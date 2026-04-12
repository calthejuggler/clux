use std::collections::HashMap;
use std::process::Command;

pub struct PaneInfo {
    pub session_name: String,
    pub target: String,
}

pub fn list_pane_targets() -> Result<HashMap<u32, PaneInfo>, Box<dyn std::error::Error>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_pid}\t#{session_name}:#{window_index}.#{pane_index}\t#{session_name}",
        ])
        .output()?;

    let mut map = HashMap::new();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.splitn(3, '\t');
        if let (Some(pid_str), Some(target), Some(session_name)) =
            (parts.next(), parts.next(), parts.next())
            && let Ok(pid) = pid_str.parse::<u32>()
        {
            let _ = map.insert(
                pid,
                PaneInfo {
                    session_name: session_name.to_owned(),
                    target: target.to_owned(),
                },
            );
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

pub fn choose_tree(filter: &str) -> Result<(), Box<dyn std::error::Error>> {
    let format_str = concat!(
        "#{?pane_format,",
        "#{pane_current_command} \"#{pane_title}\",",
        "#{?window_format,",
        "#{window_name}#{window_flags} (#{window_panes} panes)",
        "#{?#{==:#{window_panes},1}, \"#{pane_title}\",},",
        "#{session_windows} windows",
        "#{?session_grouped, (group #{session_group}: #{session_group_list}),}",
        "#{?session_attached, (attached),}",
        "#{@clux_info}",
        "}}",
    );

    let mut args = vec!["choose-tree", "-s", "-Z"];
    if filter != "all" {
        args.extend(["-f", "#{@clux_visible}"]);
    }
    args.extend(["-F", format_str]);

    let _ = Command::new("tmux").args(args).output()?;
    Ok(())
}

pub fn switch_client(target: &str) -> Result<(), Box<dyn std::error::Error>> {
    let _ = Command::new("tmux")
        .args(["switch-client", "-t", target])
        .output()?;
    let _ = Command::new("tmux")
        .args(["select-pane", "-t", target])
        .output()?;
    Ok(())
}

pub fn display_message(msg: &str) -> Result<(), Box<dyn std::error::Error>> {
    let _ = Command::new("tmux")
        .args(["display-message", msg])
        .output()?;
    Ok(())
}

pub fn display_menu(
    title: &str,
    items: &[(String, String)],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = vec!["display-menu".to_owned(), "-T".to_owned(), title.to_owned()];
    for (label, target) in items {
        args.push(label.clone());
        args.push(String::new());
        args.push(format!("switch-client -t '{target}'"));
    }
    let _ = Command::new("tmux").args(&args).output()?;
    Ok(())
}
