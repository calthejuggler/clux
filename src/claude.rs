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

pub fn detect_info(session: &ClaudeSession) -> SessionInfo {
    let default = SessionInfo {
        state: SessionState::Active,
        mode: SessionMode::Default,
        active_tasks: 0,
        active_agents: 0,
    };

    let Some(jsonl_path) = find_jsonl_path(session) else {
        return default;
    };

    let Some(tail) = read_tail_chunk(&jsonl_path) else {
        return default;
    };

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

    let (tasks, agents) = count_active_background(session);

    SessionInfo {
        state: state.unwrap_or(SessionState::Active),
        mode: mode.unwrap_or(SessionMode::Default),
        active_tasks: tasks,
        active_agents: agents,
    }
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
fn read_tail_chunk(path: &Path) -> Option<String> {
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
