use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
}

pub enum SessionState {
    Active,
    Idle,
}

pub fn discover_sessions() -> Vec<ClaudeSession> {
    let mut sessions = Vec::new();
    let sessions_dir = match home_dir().map(|h| h.join(".claude").join("sessions")) {
        Some(d) if d.is_dir() => d,
        _ => return sessions,
    };

    let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
        return sessions;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };

        let Ok(session) = serde_json::from_str::<ClaudeSession>(&contents) else {
            continue;
        };

        if !is_pid_alive(session.pid) {
            continue;
        }

        sessions.push(session);
    }

    sessions
}

pub fn detect_state(session: &ClaudeSession) -> SessionState {
    let Some(jsonl_path) = find_jsonl_path(session) else {
        return SessionState::Active;
    };

    let Some(last_line) = read_last_line(&jsonl_path) else {
        return SessionState::Active;
    };

    let Ok(entry) = serde_json::from_str::<serde_json::Value>(&last_line) else {
        return SessionState::Active;
    };

    let role = entry
        .get("message")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str());

    if role == Some("user") {
        return SessionState::Active;
    }

    let stop_reason = entry
        .get("message")
        .and_then(|m| m.get("stop_reason"))
        .and_then(|s| s.as_str());

    match stop_reason {
        Some("end_turn") => SessionState::Idle,
        _ => SessionState::Active,
    }
}

fn find_jsonl_path(session: &ClaudeSession) -> Option<PathBuf> {
    let home = home_dir()?;
    let encoded_cwd = encode_cwd(&session.cwd);
    let project_dir = home.join(".claude").join("projects").join(&encoded_cwd);
    let jsonl = project_dir.join(format!("{}.jsonl", session.session_id));
    jsonl.exists().then_some(jsonl)
}

fn encode_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
}

#[allow(clippy::verbose_file_reads)]
fn read_last_line(path: &PathBuf) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    if file_len == 0 {
        return None;
    }

    let chunk_size: u64 = 64 * 1024;
    let start = file_len.saturating_sub(chunk_size);
    let _ = file.seek(SeekFrom::Start(start)).ok()?;

    let mut buf = String::new();
    let _ = file.read_to_string(&mut buf).ok()?;

    buf.lines().last().map(ToOwned::to_owned)
}

#[cfg(target_os = "linux")]
fn is_pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "macos")]
fn is_pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}
