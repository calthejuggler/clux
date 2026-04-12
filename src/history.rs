use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::claude::ClaudeSession;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    display: Option<String>,
    session_id: Option<String>,
    project: Option<String>,
    timestamp: Option<u64>,
}

pub struct SessionSummary {
    pub last_display: String,
    pub last_timestamp: u64,
    pub first_timestamp: u64,
    pub project: String,
    pub session_changed: bool,
}

pub struct Summary {
    pub display: String,
    pub timestamp: u64,
}

pub struct ChainResult {
    pub summary: Summary,
    pub depth: usize,
    pub target_sid: String,
}

pub fn load_summaries(sessions: &[ClaudeSession]) -> HashMap<u32, Summary> {
    let Some(contents) = history_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return HashMap::new();
    };
    load_summaries_from(sessions, &contents)
}

pub fn load_summaries_from(sessions: &[ClaudeSession], contents: &str) -> HashMap<u32, Summary> {
    let mut result = HashMap::new();

    let mut by_session_id: HashMap<String, SessionSummary> = HashMap::new();

    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<HistoryEntry>(line) else {
            continue;
        };

        let (Some(session_id), Some(display), Some(project), Some(ts)) = (
            entry.session_id,
            entry.display,
            entry.project,
            entry.timestamp,
        ) else {
            continue;
        };

        let truncated = truncate_display(&display, 80);
        let is_clear = display == "/clear";

        let _ = by_session_id
            .entry(session_id)
            .and_modify(|s| {
                s.last_display.clone_from(&truncated);
                s.last_timestamp = ts;
                if is_clear {
                    s.session_changed = true;
                }
            })
            .or_insert(SessionSummary {
                last_display: truncated,
                last_timestamp: ts,
                first_timestamp: ts,
                project,
                session_changed: is_clear,
            });
    }

    let live_sids: std::collections::HashSet<String> =
        sessions.iter().map(|s| s.session_id.clone()).collect();

    let mut candidates: Vec<(u32, ChainResult)> = sessions
        .iter()
        .filter_map(|session| {
            let chain = follow_chain(
                &session.session_id,
                &session.cwd,
                &by_session_id,
                &live_sids,
            );
            chain.map(|c| (session.pid, c))
        })
        .collect();

    candidates.sort_by(|(_, a), (_, b)| b.depth.cmp(&a.depth));

    let mut assigned_targets: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (pid, chain) in candidates {
        if assigned_targets.contains(&chain.target_sid) {
            continue;
        }
        let _ = assigned_targets.insert(chain.target_sid);
        let _ = result.insert(pid, chain.summary);
    }

    result
}

pub fn follow_chain(
    start_sid: &str,
    cwd: &str,
    map: &HashMap<String, SessionSummary>,
    live_sids: &std::collections::HashSet<String>,
) -> Option<ChainResult> {
    let mut current_sid = start_sid;
    for (depth, _) in (0..20).enumerate() {
        let summary = map.get(current_sid)?;

        if !summary.session_changed {
            return Some(ChainResult {
                summary: Summary {
                    display: summary.last_display.clone(),
                    timestamp: summary.last_timestamp,
                },
                depth,
                target_sid: current_sid.to_owned(),
            });
        }

        let after = summary.last_timestamp;
        let next = map
            .iter()
            .filter(|(sid, s)| {
                (sid.as_str() == start_sid || !live_sids.contains(sid.as_str()))
                    && s.project == cwd
                    && s.first_timestamp > after
            })
            .min_by_key(|(_, s)| s.first_timestamp);

        let (next_sid, _) = next?;

        current_sid = next_sid;
    }

    None
}

pub fn truncate_display(text: &str, max_chars: usize) -> String {
    let trimmed = text.lines().next().unwrap_or(text);
    let char_count = trimmed.chars().count();
    if char_count <= max_chars {
        return trimmed.to_owned();
    }
    let mut result: String = trimmed.chars().take(max_chars).collect();
    result.push_str("...");
    result
}

fn history_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".claude").join("history.jsonl");
    path.exists().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_display_short() {
        assert_eq!(truncate_display("hello", 10), "hello");
    }

    #[test]
    fn truncate_display_long() {
        let result = truncate_display("this is a very long description", 10);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 13);
    }

    #[test]
    fn truncate_display_multiline() {
        let result = truncate_display("first line\nsecond line\nthird line", 80);
        assert_eq!(result, "first line");
    }

    #[test]
    fn load_summaries_from_empty() {
        let sessions: Vec<ClaudeSession> = Vec::new();
        let result = load_summaries_from(&sessions, "");
        assert!(result.is_empty());
    }

    #[test]
    fn load_summaries_from_basic() {
        let sessions = vec![ClaudeSession {
            pid: 100,
            session_id: "sess-1".to_owned(),
            cwd: "/home/user/project".to_owned(),
            started_at: 1700000000,
        }];

        let history = serde_json::json!({
            "display": "Fix the login bug",
            "sessionId": "sess-1",
            "project": "/home/user/project",
            "timestamp": 1700000100_u64
        });

        let result = load_summaries_from(&sessions, &history.to_string());
        assert_eq!(result.len(), 1);
        let summary = result.get(&100).expect("summary for pid 100");
        assert_eq!(summary.display, "Fix the login bug");
        assert_eq!(summary.timestamp, 1700000100);
    }

    #[test]
    fn load_summaries_from_clear_follows_chain() {
        let sessions = vec![ClaudeSession {
            pid: 100,
            session_id: "sess-1".to_owned(),
            cwd: "/home/user/project".to_owned(),
            started_at: 1700000000,
        }];

        let lines = vec![
            serde_json::json!({
                "display": "First task",
                "sessionId": "sess-1",
                "project": "/home/user/project",
                "timestamp": 1700000100_u64
            }),
            serde_json::json!({
                "display": "/clear",
                "sessionId": "sess-1",
                "project": "/home/user/project",
                "timestamp": 1700000200_u64
            }),
            serde_json::json!({
                "display": "Second task after clear",
                "sessionId": "sess-2",
                "project": "/home/user/project",
                "timestamp": 1700000300_u64
            }),
        ];

        let contents: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let result = load_summaries_from(&sessions, &contents);
        assert_eq!(result.len(), 1);
        let summary = result.get(&100).expect("summary for pid 100");
        assert_eq!(summary.display, "Second task after clear");
    }

    #[test]
    fn load_summaries_from_missing_fields_skipped() {
        let sessions = vec![ClaudeSession {
            pid: 100,
            session_id: "sess-1".to_owned(),
            cwd: "/tmp".to_owned(),
            started_at: 0,
        }];

        let contents = r#"{"display":"hello"}
{"sessionId":"sess-1"}
not json at all"#;

        let result = load_summaries_from(&sessions, contents);
        assert!(result.is_empty());
    }

    #[test]
    fn follow_chain_no_clear() {
        let mut map = HashMap::new();
        let _ = map.insert(
            "sess-1".to_owned(),
            SessionSummary {
                last_display: "Working on feature".to_owned(),
                last_timestamp: 100,
                first_timestamp: 50,
                project: "/project".to_owned(),
                session_changed: false,
            },
        );
        let live = std::collections::HashSet::from(["sess-1".to_owned()]);

        let result = follow_chain("sess-1", "/project", &map, &live);
        assert!(result.is_some());
        let chain = result.expect("chain");
        assert_eq!(chain.summary.display, "Working on feature");
        assert_eq!(chain.depth, 0);
    }

    #[test]
    fn follow_chain_with_clear() {
        let mut map = HashMap::new();
        let _ = map.insert(
            "sess-1".to_owned(),
            SessionSummary {
                last_display: "/clear".to_owned(),
                last_timestamp: 100,
                first_timestamp: 50,
                project: "/project".to_owned(),
                session_changed: true,
            },
        );
        let _ = map.insert(
            "sess-2".to_owned(),
            SessionSummary {
                last_display: "New work".to_owned(),
                last_timestamp: 200,
                first_timestamp: 150,
                project: "/project".to_owned(),
                session_changed: false,
            },
        );
        let live = std::collections::HashSet::from(["sess-1".to_owned()]);

        let result = follow_chain("sess-1", "/project", &map, &live);
        assert!(result.is_some());
        let chain = result.expect("chain");
        assert_eq!(chain.summary.display, "New work");
        assert_eq!(chain.depth, 1);
    }

    #[test]
    fn follow_chain_missing_session() {
        let map = HashMap::new();
        let live = std::collections::HashSet::new();
        let result = follow_chain("nonexistent", "/project", &map, &live);
        assert!(result.is_none());
    }

    #[test]
    fn load_summaries_from_multiple_sessions() {
        let sessions = vec![
            ClaudeSession {
                pid: 100,
                session_id: "sess-1".to_owned(),
                cwd: "/project-a".to_owned(),
                started_at: 1700000000,
            },
            ClaudeSession {
                pid: 200,
                session_id: "sess-2".to_owned(),
                cwd: "/project-b".to_owned(),
                started_at: 1700000000,
            },
        ];

        let lines = vec![
            serde_json::json!({
                "display": "Task A",
                "sessionId": "sess-1",
                "project": "/project-a",
                "timestamp": 1700000100_u64
            }),
            serde_json::json!({
                "display": "Task B",
                "sessionId": "sess-2",
                "project": "/project-b",
                "timestamp": 1700000200_u64
            }),
        ];

        let contents: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let result = load_summaries_from(&sessions, &contents);
        assert_eq!(result.len(), 2);
        assert_eq!(result.get(&100).expect("pid 100").display, "Task A");
        assert_eq!(result.get(&200).expect("pid 200").display, "Task B");
    }

    #[test]
    fn load_summaries_from_last_display_wins() {
        let sessions = vec![ClaudeSession {
            pid: 100,
            session_id: "sess-1".to_owned(),
            cwd: "/project".to_owned(),
            started_at: 1700000000,
        }];

        let lines = vec![
            serde_json::json!({
                "display": "First prompt",
                "sessionId": "sess-1",
                "project": "/project",
                "timestamp": 1700000100_u64
            }),
            serde_json::json!({
                "display": "Second prompt",
                "sessionId": "sess-1",
                "project": "/project",
                "timestamp": 1700000200_u64
            }),
        ];

        let contents: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let result = load_summaries_from(&sessions, &contents);
        let summary = result.get(&100).expect("pid 100");
        assert_eq!(summary.display, "Second prompt");
        assert_eq!(summary.timestamp, 1700000200);
    }

    #[test]
    fn follow_chain_different_project_does_not_chain() {
        let mut map = HashMap::new();
        let _ = map.insert(
            "sess-1".to_owned(),
            SessionSummary {
                last_display: "/clear".to_owned(),
                last_timestamp: 100,
                first_timestamp: 50,
                project: "/project-a".to_owned(),
                session_changed: true,
            },
        );
        let _ = map.insert(
            "sess-2".to_owned(),
            SessionSummary {
                last_display: "Work in project B".to_owned(),
                last_timestamp: 200,
                first_timestamp: 150,
                project: "/project-b".to_owned(),
                session_changed: false,
            },
        );
        let live = std::collections::HashSet::from(["sess-1".to_owned()]);

        let result = follow_chain("sess-1", "/project-a", &map, &live);
        assert!(result.is_none());
    }

    #[test]
    fn load_summaries_deduplicates_chain_targets() {
        let sessions = vec![
            ClaudeSession {
                pid: 100,
                session_id: "sess-1".to_owned(),
                cwd: "/project".to_owned(),
                started_at: 1700000000,
            },
            ClaudeSession {
                pid: 200,
                session_id: "sess-2".to_owned(),
                cwd: "/project".to_owned(),
                started_at: 1700000000,
            },
        ];

        let lines = vec![
            serde_json::json!({
                "display": "Only entry",
                "sessionId": "sess-1",
                "project": "/project",
                "timestamp": 1700000100_u64
            }),
            serde_json::json!({
                "display": "Only entry for sess-2",
                "sessionId": "sess-2",
                "project": "/project",
                "timestamp": 1700000200_u64
            }),
        ];

        let contents: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let result = load_summaries_from(&sessions, &contents);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn follow_chain_max_depth_terminates() {
        let mut map = HashMap::new();
        for idx in 0..25 {
            let _ = map.insert(
                format!("sess-{idx}"),
                SessionSummary {
                    last_display: "/clear".to_owned(),
                    last_timestamp: u64::try_from(idx * 100 + 50).unwrap_or(0),
                    first_timestamp: u64::try_from(idx * 100).unwrap_or(0),
                    project: "/project".to_owned(),
                    session_changed: true,
                },
            );
        }
        let live = std::collections::HashSet::from(["sess-0".to_owned()]);

        let result = follow_chain("sess-0", "/project", &map, &live);
        assert!(result.is_none());
    }
}
