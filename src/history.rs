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
}

pub struct Summary {
    pub display: String,
    pub timestamp: u64,
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

        let _ = by_session_id
            .entry(session_id)
            .and_modify(|s| {
                s.last_display.clone_from(&truncated);
                s.last_timestamp = ts;
            })
            .or_insert(SessionSummary {
                last_display: truncated,
                last_timestamp: ts,
                first_timestamp: ts,
                project,
            });
    }

    let claimed: std::collections::HashSet<&str> =
        sessions.iter().map(|s| s.session_id.as_str()).collect();

    for session in sessions {
        let (mut best_ts, mut best_display) = by_session_id
            .get(&session.session_id)
            .map_or((0_u64, None), |summary| {
                (summary.last_timestamp, Some(summary.last_display.clone()))
            });

        for (sid, summary) in &by_session_id {
            if claimed.contains(sid.as_str()) {
                continue;
            }

            if summary.project != session.cwd || summary.last_timestamp <= best_ts {
                continue;
            }

            if summary.first_timestamp < session.started_at {
                continue;
            }

            let range = (session.started_at + 1)..=summary.first_timestamp;
            let dominated_by_other = sessions.iter().any(|other| {
                other.pid != session.pid
                    && other.cwd == session.cwd
                    && range.contains(&other.started_at)
            });

            if dominated_by_other {
                continue;
            }

            best_ts = summary.last_timestamp;
            best_display = Some(summary.last_display.clone());
        }

        if let Some(display) = best_display {
            let _ = result.insert(
                session.pid,
                Summary {
                    display,
                    timestamp: best_ts,
                },
            );
        }
    }

    result
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
