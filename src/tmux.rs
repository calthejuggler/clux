use std::collections::HashMap;
use std::process::Command;

pub fn list_panes() -> Result<HashMap<u32, String>, Box<dyn std::error::Error>> {
    let output = Command::new("tmux")
        .args(["list-panes", "-a", "-F", "#{pane_pid} #{session_name}"])
        .output()?;

    let mut map = HashMap::new();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some((pid_str, session_name)) = line.split_once(' ')
            && let Ok(pid) = pid_str.parse::<u32>()
        {
            let _ = map.insert(pid, session_name.to_owned());
        }
    }
    Ok(map)
}

pub fn list_sessions() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().map(ToOwned::to_owned).collect())
}

pub fn set_session_option(
    session: &str,
    key: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let target = format!("{session}:");
    let _ = Command::new("tmux")
        .args(["set", "-t", &target, key, value])
        .output()?;
    Ok(())
}

pub fn unset_session_option(session: &str, key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let target = format!("{session}:");
    let _ = Command::new("tmux")
        .args(["set", "-t", &target, "-u", key])
        .output()?;
    Ok(())
}

pub fn get_global_option(key: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let output = Command::new("tmux")
        .args(["show-option", "-gqv", key])
        .output()?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}
