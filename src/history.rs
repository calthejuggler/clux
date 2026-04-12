use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    display: Option<String>,
    session_id: Option<String>,
}

pub fn load_first_messages() -> HashMap<String, String> {
    let mut messages = HashMap::new();

    let Some(path) = history_path() else {
        return messages;
    };

    let Ok(contents) = std::fs::read_to_string(&path) else {
        return messages;
    };

    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<HistoryEntry>(line) else {
            continue;
        };

        let (Some(session_id), Some(display)) = (entry.session_id, entry.display) else {
            continue;
        };

        if messages.contains_key(&session_id) {
            continue;
        }

        let truncated = truncate_display(&display, 80);
        let _ = messages.insert(session_id, truncated);
    }

    messages
}

fn truncate_display(text: &str, max_len: usize) -> String {
    let trimmed = text.lines().next().unwrap_or(text);
    if trimmed.len() <= max_len {
        return trimmed.to_owned();
    }
    let mut end = max_len;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = trimmed.get(..end).unwrap_or(trimmed).to_owned();
    result.push_str("...");
    result
}

fn history_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".claude").join("history.jsonl");
    path.exists().then_some(path)
}
