use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub started_at: u64,
}

pub enum SessionState {
    Active,
    Idle,
}

pub enum SessionMode {
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
}

pub struct SessionInfo {
    pub state: SessionState,
    pub mode: SessionMode,
    pub active_tasks: u32,
    pub active_agents: u32,
}

pub fn discover_sessions() -> Vec<ClaudeSession> {
    let sessions_dir = match home_dir().map(|h| h.join(".claude").join("sessions")) {
        Some(d) if d.is_dir() => d,
        _ => return Vec::new(),
    };
    discover_sessions_in(&sessions_dir, true)
}

pub fn discover_sessions_in(sessions_dir: &Path, check_alive: bool) -> Vec<ClaudeSession> {
    let mut sessions = Vec::new();

    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
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

        if check_alive && !is_pid_alive(session.pid) {
            continue;
        }

        sessions.push(session);
    }

    sessions
}

pub fn detect_info(session: &ClaudeSession) -> SessionInfo {
    let Some(home) = home_dir() else {
        return SessionInfo {
            state: SessionState::Active,
            mode: SessionMode::Default,
            active_tasks: 0,
            active_agents: 0,
        };
    };
    detect_info_in(session, &home)
}

pub fn detect_info_in(session: &ClaudeSession, home: &Path) -> SessionInfo {
    let default = SessionInfo {
        state: SessionState::Active,
        mode: SessionMode::Default,
        active_tasks: 0,
        active_agents: 0,
    };

    let Some(jsonl_path) = find_jsonl_path_in(session, home) else {
        return default;
    };

    let Some(tail) = read_tail_chunk(&jsonl_path) else {
        return default;
    };

    let (state, mode) = parse_jsonl_tail(&tail);

    let (tasks, agents) = count_active_background(session);

    let mut final_state = state;

    if matches!(final_state, SessionState::Idle)
        && is_jsonl_stale(session, &jsonl_path)
        && has_child_processes(session.pid)
    {
        final_state = SessionState::Active;
    }

    SessionInfo {
        state: final_state,
        mode,
        active_tasks: tasks,
        active_agents: agents,
    }
}

pub fn parse_jsonl_tail(tail: &str) -> (SessionState, SessionMode) {
    let mut state = None;
    let mut mode = None;

    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        if mode.is_none()
            && let Some(pm) = entry.get("permissionMode").and_then(|m| m.as_str())
        {
            mode = Some(match pm {
                "plan" => SessionMode::Plan,
                "bypassPermissions" => SessionMode::BypassPermissions,
                "acceptEdits" => SessionMode::AcceptEdits,
                _ => SessionMode::Default,
            });
        }

        if state.is_none() {
            let entry_type = entry.get("type").and_then(|t| t.as_str());
            if entry_type == Some("user") || entry_type == Some("assistant") {
                let role = entry
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str());

                if role == Some("user") {
                    state = Some(SessionState::Active);
                } else {
                    let stop_reason = entry
                        .get("message")
                        .and_then(|m| m.get("stop_reason"))
                        .and_then(|s| s.as_str());

                    state = Some(match stop_reason {
                        Some("end_turn") => SessionState::Idle,
                        _ => SessionState::Active,
                    });
                }
            }
        }

        if state.is_some() && mode.is_some() {
            break;
        }
    }

    (
        state.unwrap_or(SessionState::Active),
        mode.unwrap_or(SessionMode::Default),
    )
}

fn is_jsonl_stale(session: &ClaudeSession, jsonl_path: &Path) -> bool {
    let Some(jsonl_mtime) = jsonl_path.metadata().ok().and_then(|m| m.modified().ok()) else {
        return false;
    };

    let age = std::time::SystemTime::now()
        .duration_since(jsonl_mtime)
        .ok()
        .map_or(0, |d| d.as_secs());

    if age < 60 {
        return false;
    }

    let Some(home) = home_dir() else {
        return false;
    };
    let encoded_cwd = encode_cwd(&session.cwd);
    let project_dir = home.join(".claude").join("projects").join(&encoded_cwd);

    let Ok(entries) = std::fs::read_dir(&project_dir) else {
        return false;
    };

    entries.flatten().any(|entry| {
        let path = entry.path();
        path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && path != *jsonl_path
            && path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|t| {
                    t.duration_since(jsonl_mtime)
                        .ok()
                        .is_some_and(|gap| gap.as_secs() > 30)
                })
    })
}

#[cfg(target_os = "linux")]
fn has_child_processes(pid: u32) -> bool {
    let children_path = format!("/proc/{pid}/task/{pid}/children");
    std::fs::read_to_string(&children_path)
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
}

#[cfg(target_os = "macos")]
fn has_child_processes(pid: u32) -> bool {
    std::process::Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .ok()
        .is_some_and(|o| !o.stdout.is_empty())
}

fn count_active_background(session: &ClaudeSession) -> (u32, u32) {
    let uid = current_uid();
    let encoded_cwd = encode_cwd(&session.cwd);
    let tasks_dir = PathBuf::from(format!(
        "/tmp/claude-{uid}/{encoded_cwd}/{}/tasks",
        session.session_id
    ));

    let Ok(entries) = std::fs::read_dir(&tasks_dir) else {
        return (0, 0);
    };

    let open_files = collect_open_files(session.pid);

    let mut tasks = 0;
    let mut agents = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("output") {
            continue;
        }
        let is_active = path
            .canonicalize()
            .ok()
            .is_some_and(|canonical| open_files.contains(&canonical));
        if !is_active {
            continue;
        }
        if path.is_symlink() {
            agents += 1;
        } else {
            tasks += 1;
        }
    }
    (tasks, agents)
}

#[cfg(target_os = "linux")]
fn collect_open_files(parent_pid: u32) -> std::collections::HashSet<PathBuf> {
    let mut files = std::collections::HashSet::new();
    let child_pids = child_pids_of(parent_pid);
    for pid in child_pids {
        let fd_dir = format!("/proc/{pid}/fd");
        if let Ok(fds) = std::fs::read_dir(&fd_dir) {
            for fd in fds.flatten() {
                if let Ok(target) = std::fs::read_link(fd.path()) {
                    let _ = files.insert(target);
                }
            }
        }
    }
    files
}

#[cfg(target_os = "linux")]
fn child_pids_of(parent: u32) -> Vec<u32> {
    let task_dir = format!("/proc/{parent}/task");
    let Ok(entries) = std::fs::read_dir(&task_dir) else {
        return Vec::new();
    };

    let mut children = Vec::new();
    for entry in entries.flatten() {
        let children_path = entry.path().join("children");
        if let Ok(content) = std::fs::read_to_string(&children_path) {
            for pid_str in content.split_whitespace() {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    children.push(pid);
                    children.extend(descendant_pids(pid));
                }
            }
        }
    }
    children
}

#[cfg(target_os = "linux")]
fn descendant_pids(pid: u32) -> Vec<u32> {
    let children_path = format!("/proc/{pid}/task/{pid}/children");
    let Ok(content) = std::fs::read_to_string(&children_path) else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    for pid_str in content.split_whitespace() {
        if let Ok(child) = pid_str.parse::<u32>() {
            pids.push(child);
            pids.extend(descendant_pids(child));
        }
    }
    pids
}

#[cfg(target_os = "macos")]
fn descendant_pids(pid: u32) -> Vec<u32> {
    let Ok(output) = std::process::Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    for pid_str in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        if let Ok(child) = pid_str.parse::<u32>() {
            pids.push(child);
            pids.extend(descendant_pids(child));
        }
    }
    pids
}

#[cfg(target_os = "macos")]
fn collect_open_files(parent_pid: u32) -> std::collections::HashSet<PathBuf> {
    let child_pids = descendant_pids(parent_pid);
    let mut files = std::collections::HashSet::new();
    for pid in child_pids {
        if let Ok(lsof) = std::process::Command::new("lsof")
            .args(["-p", &pid.to_string(), "-Fn"])
            .output()
        {
            for line in String::from_utf8_lossy(&lsof.stdout).lines() {
                if let Some(path) = line.strip_prefix('n') {
                    let _ = files.insert(PathBuf::from(path));
                }
            }
        }
    }
    files
}

pub fn find_jsonl_path_in(session: &ClaudeSession, home: &Path) -> Option<PathBuf> {
    let encoded_cwd = encode_cwd(&session.cwd);
    let project_dir = home.join(".claude").join("projects").join(&encoded_cwd);
    let jsonl = project_dir.join(format!("{}.jsonl", session.session_id));
    jsonl.exists().then_some(jsonl)
}

pub fn encode_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
}

#[allow(clippy::verbose_file_reads)]
pub fn read_tail_chunk(path: &Path) -> Option<String> {
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

    if start > 0
        && let Some(newline_pos) = buf.find('\n')
    {
        let _ = buf.drain(..=newline_pos);
    }

    Some(buf)
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

#[cfg(target_os = "linux")]
fn current_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|u| u.parse().ok())
        })
        .unwrap_or(1000)
}

#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(501)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_cwd_basic() {
        assert_eq!(encode_cwd("/home/user/project"), "-home-user-project");
    }

    #[test]
    fn encode_cwd_root() {
        assert_eq!(encode_cwd("/"), "-");
    }

    #[test]
    fn encode_cwd_trailing_slash() {
        assert_eq!(encode_cwd("/home/user/"), "-home-user-");
    }

    #[test]
    fn discover_sessions_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions = discover_sessions_in(dir.path(), false);
        assert!(sessions.is_empty());
    }

    #[test]
    fn discover_sessions_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("bad.json"), "not json").expect("write");
        let sessions = discover_sessions_in(dir.path(), false);
        assert!(sessions.is_empty());
    }

    #[test]
    fn discover_sessions_non_json_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("notes.txt"), "hello").expect("write");
        std::fs::write(dir.path().join("data.log"), "log line").expect("write");
        let sessions = discover_sessions_in(dir.path(), false);
        assert!(sessions.is_empty());
    }

    #[test]
    fn discover_sessions_valid_no_pid_check() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json = serde_json::json!({
            "pid": 99999,
            "sessionId": "abc-123",
            "cwd": "/home/user/project",
            "startedAt": 1700000000_u64
        });
        std::fs::write(dir.path().join("sess.json"), json.to_string()).expect("write");
        let sessions = discover_sessions_in(dir.path(), false);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions.first().expect("session").session_id, "abc-123");
        assert_eq!(sessions.first().expect("session").cwd, "/home/user/project");
    }

    #[test]
    fn discover_sessions_skips_dead_pids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json = serde_json::json!({
            "pid": 4294967295_u64,
            "sessionId": "dead-session",
            "cwd": "/tmp",
            "startedAt": 1700000000_u64
        });
        std::fs::write(dir.path().join("dead.json"), json.to_string()).expect("write");
        let sessions = discover_sessions_in(dir.path(), true);
        assert!(sessions.is_empty());
    }

    #[test]
    fn read_tail_chunk_small_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("small.jsonl");
        std::fs::write(&path, "line1\nline2\nline3\n").expect("write");
        let result = read_tail_chunk(&path);
        assert!(result.is_some());
        let content = result.expect("content");
        assert!(content.contains("line1"));
        assert!(content.contains("line3"));
    }

    #[test]
    fn read_tail_chunk_empty_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.jsonl");
        std::fs::write(&path, "").expect("write");
        assert!(read_tail_chunk(&path).is_none());
    }

    #[test]
    fn read_tail_chunk_large_file_drops_partial_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("large.jsonl");
        let line = "a".repeat(1000);
        let mut content = String::new();
        for _ in 0..100 {
            content.push_str(&line);
            content.push('\n');
        }
        std::fs::write(&path, &content).expect("write");
        let result = read_tail_chunk(&path);
        assert!(result.is_some());
        let tail = result.expect("tail");
        for l in tail.lines() {
            assert!(l.len() == 1000 || l.is_empty());
        }
    }

    #[test]
    fn parse_jsonl_tail_user_message_last() {
        let tail = r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn"}}
{"type":"user","message":{"role":"user"}}"#;
        let (state, _) = parse_jsonl_tail(tail);
        assert!(matches!(state, SessionState::Active));
    }

    #[test]
    fn parse_jsonl_tail_assistant_end_turn() {
        let tail =
            r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn"}}"#;
        let (state, _) = parse_jsonl_tail(tail);
        assert!(matches!(state, SessionState::Idle));
    }

    #[test]
    fn parse_jsonl_tail_assistant_no_stop_reason() {
        let tail = r#"{"type":"assistant","message":{"role":"assistant"}}"#;
        let (state, _) = parse_jsonl_tail(tail);
        assert!(matches!(state, SessionState::Active));
    }

    #[test]
    fn parse_jsonl_tail_empty() {
        let (state, mode) = parse_jsonl_tail("");
        assert!(matches!(state, SessionState::Active));
        assert!(matches!(mode, SessionMode::Default));
    }

    #[test]
    fn parse_jsonl_tail_permission_plan() {
        let tail = r#"{"permissionMode":"plan"}
{"type":"user","message":{"role":"user"}}"#;
        let (_, mode) = parse_jsonl_tail(tail);
        assert!(matches!(mode, SessionMode::Plan));
    }

    #[test]
    fn parse_jsonl_tail_permission_bypass() {
        let tail = r#"{"permissionMode":"bypassPermissions"}
{"type":"user","message":{"role":"user"}}"#;
        let (_, mode) = parse_jsonl_tail(tail);
        assert!(matches!(mode, SessionMode::BypassPermissions));
    }

    #[test]
    fn parse_jsonl_tail_permission_accept_edits() {
        let tail = r#"{"permissionMode":"acceptEdits"}
{"type":"user","message":{"role":"user"}}"#;
        let (_, mode) = parse_jsonl_tail(tail);
        assert!(matches!(mode, SessionMode::AcceptEdits));
    }

    #[test]
    fn parse_jsonl_tail_permission_default() {
        let tail = r#"{"permissionMode":"default"}
{"type":"user","message":{"role":"user"}}"#;
        let (_, mode) = parse_jsonl_tail(tail);
        assert!(matches!(mode, SessionMode::Default));
    }

    #[test]
    fn parse_jsonl_tail_permission_unknown() {
        let tail = r#"{"permissionMode":"somethingNew"}
{"type":"user","message":{"role":"user"}}"#;
        let (_, mode) = parse_jsonl_tail(tail);
        assert!(matches!(mode, SessionMode::Default));
    }

    fn make_session(session_id: &str, cwd: &str) -> ClaudeSession {
        ClaudeSession {
            pid: 1,
            session_id: session_id.to_owned(),
            cwd: cwd.to_owned(),
            started_at: 0,
        }
    }

    fn setup_jsonl(home: &Path, cwd: &str, session_id: &str, content: &str) {
        let encoded = encode_cwd(cwd);
        let project_dir = home.join(".claude").join("projects").join(&encoded);
        std::fs::create_dir_all(&project_dir).expect("mkdir");
        std::fs::write(project_dir.join(format!("{session_id}.jsonl")), content).expect("write");
    }

    #[test]
    fn detect_info_no_jsonl_returns_active() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("no-file", "/home/user");
        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.state, SessionState::Active));
        assert!(matches!(info.mode, SessionMode::Default));
        assert_eq!(info.active_tasks, 0);
        assert_eq!(info.active_agents, 0);
    }

    #[test]
    fn detect_info_idle_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("sess-idle", "/home/user");
        let content =
            r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn"}}"#;
        setup_jsonl(dir.path(), "/home/user", "sess-idle", content);

        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.state, SessionState::Idle));
    }

    #[test]
    fn detect_info_active_user_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("sess-active", "/home/user");
        let content = r#"{"type":"user","message":{"role":"user"}}"#;
        setup_jsonl(dir.path(), "/home/user", "sess-active", content);

        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.state, SessionState::Active));
    }

    #[test]
    fn detect_info_active_no_stop_reason() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("sess-nostop", "/home/user");
        let content = r#"{"type":"assistant","message":{"role":"assistant"}}"#;
        setup_jsonl(dir.path(), "/home/user", "sess-nostop", content);

        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.state, SessionState::Active));
    }

    #[test]
    fn detect_info_plan_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("sess-plan", "/home/user");
        let content =
            "{\"permissionMode\":\"plan\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\"}}";
        setup_jsonl(dir.path(), "/home/user", "sess-plan", content);

        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.mode, SessionMode::Plan));
    }

    #[test]
    fn detect_info_bypass_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("sess-yolo", "/home/user");
        let content = "{\"permissionMode\":\"bypassPermissions\"}\n{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"stop_reason\":\"end_turn\"}}";
        setup_jsonl(dir.path(), "/home/user", "sess-yolo", content);

        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.mode, SessionMode::BypassPermissions));
        assert!(matches!(info.state, SessionState::Idle));
    }

    #[test]
    fn detect_info_accept_edits_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("sess-edits", "/home/user");
        let content = "{\"permissionMode\":\"acceptEdits\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\"}}";
        setup_jsonl(dir.path(), "/home/user", "sess-edits", content);

        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.mode, SessionMode::AcceptEdits));
    }

    #[test]
    fn parse_jsonl_tail_mixed_garbage_and_valid() {
        let tail =
            "not json at all\n{invalid json}\n{\"type\":\"user\",\"message\":{\"role\":\"user\"}}";
        let (state, _) = parse_jsonl_tail(tail);
        assert!(matches!(state, SessionState::Active));
    }

    #[test]
    fn parse_jsonl_tail_only_garbage() {
        let tail = "garbage line 1\n{broken\nmore garbage";
        let (state, mode) = parse_jsonl_tail(tail);
        assert!(matches!(state, SessionState::Active));
        assert!(matches!(mode, SessionMode::Default));
    }

    #[test]
    fn parse_jsonl_tail_multiple_messages_uses_last() {
        let tail = "{\"type\":\"user\",\"message\":{\"role\":\"user\"}}\n{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"stop_reason\":\"end_turn\"}}";
        let (state, _) = parse_jsonl_tail(tail);
        assert!(matches!(state, SessionState::Idle));
    }

    #[test]
    fn parse_jsonl_tail_mode_on_different_line_than_state() {
        let tail = "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"stop_reason\":\"end_turn\"}}\n{\"permissionMode\":\"plan\"}";
        let (state, mode) = parse_jsonl_tail(tail);
        assert!(matches!(state, SessionState::Idle));
        assert!(matches!(mode, SessionMode::Plan));
    }

    #[test]
    fn discover_sessions_multiple_valid() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 1..=3 {
            let json = serde_json::json!({
                "pid": 99990 + i,
                "sessionId": format!("sess-{i}"),
                "cwd": "/home/user/project",
                "startedAt": 1700000000_u64
            });
            std::fs::write(dir.path().join(format!("sess-{i}.json")), json.to_string())
                .expect("write");
        }
        let sessions = discover_sessions_in(dir.path(), false);
        assert_eq!(sessions.len(), 3);
    }

    #[test]
    fn detect_info_empty_jsonl_returns_active() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = make_session("sess-empty", "/home/user");
        setup_jsonl(dir.path(), "/home/user", "sess-empty", "");

        let info = detect_info_in(&session, dir.path());
        assert!(matches!(info.state, SessionState::Active));
    }

    #[test]
    fn find_jsonl_path_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let projects_dir = dir
            .path()
            .join(".claude")
            .join("projects")
            .join("-home-user");
        std::fs::create_dir_all(&projects_dir).expect("mkdir");
        std::fs::write(projects_dir.join("sess-1.jsonl"), "data").expect("write");

        let session = ClaudeSession {
            pid: 1,
            session_id: "sess-1".to_owned(),
            cwd: "/home/user".to_owned(),
            started_at: 0,
        };

        let result = find_jsonl_path_in(&session, dir.path());
        assert!(result.is_some());
    }

    #[test]
    fn find_jsonl_path_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = ClaudeSession {
            pid: 1,
            session_id: "nonexistent".to_owned(),
            cwd: "/home/user".to_owned(),
            started_at: 0,
        };
        let result = find_jsonl_path_in(&session, dir.path());
        assert!(result.is_none());
    }
}
