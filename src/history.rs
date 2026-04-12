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

struct SessionSummary {
    last_display: String,
    last_timestamp: u64,
    first_timestamp: u64,
    project: String,
    session_changed: bool,
}

pub struct Summary {
    pub display: String,
    pub timestamp: u64,
}

struct ChainResult {
    summary: Summary,
    depth: usize,
    target_sid: String,
}

pub fn load_summaries(sessions: &[ClaudeSession]) -> HashMap<u32, Summary> {
    let mut result = HashMap::new();

    let Some(contents) = history_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return result;
    };

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

fn follow_chain(
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

fn truncate_display(text: &str, max_chars: usize) -> String {
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
