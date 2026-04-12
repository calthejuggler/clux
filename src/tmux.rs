use std::collections::HashMap;
use std::process::Command;

pub struct PaneInfo {
    pub session_name: String,
    pub target: String,
}

pub fn list_pane_targets() -> anyhow::Result<HashMap<u32, PaneInfo>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_pid}\t#{session_name}:#{window_index}.#{pane_index}\t#{session_name}",
        ])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_pane_targets(&stdout))
}

pub fn parse_pane_targets(stdout: &str) -> HashMap<u32, PaneInfo> {
    let mut map = HashMap::new();
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
    map
}

pub fn list_sessions() -> anyhow::Result<Vec<String>> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_sessions(&stdout))
}

pub fn parse_sessions(stdout: &str) -> Vec<String> {
    stdout.lines().map(ToOwned::to_owned).collect()
}

pub fn set_session_option(session: &str, key: &str, value: &str) -> anyhow::Result<()> {
    let target = format!("{session}:");
    let _ = Command::new("tmux")
        .args(["set", "-t", &target, key, value])
        .output()?;
    Ok(())
}

pub fn unset_session_option(session: &str, key: &str) -> anyhow::Result<()> {
    let target = format!("{session}:");
    let _ = Command::new("tmux")
        .args(["set", "-t", &target, "-u", key])
        .output()?;
    Ok(())
}

pub fn get_global_option(key: &str) -> anyhow::Result<Option<String>> {
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

pub fn choose_tree(filter: &str) -> anyhow::Result<()> {
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

pub fn switch_client(target: &str) -> anyhow::Result<()> {
    let _ = Command::new("tmux")
        .args(["switch-client", "-t", target])
        .output()?;
    let _ = Command::new("tmux")
        .args(["select-pane", "-t", target])
        .output()?;
    Ok(())
}

pub fn display_message(msg: &str) -> anyhow::Result<()> {
    let _ = Command::new("tmux")
        .args(["display-message", msg])
        .output()?;
    Ok(())
}

pub fn display_menu(title: &str, items: &[(String, String)]) -> anyhow::Result<()> {
    let mut args: Vec<String> = vec!["display-menu".to_owned(), "-T".to_owned(), title.to_owned()];
    for (label, target) in items {
        args.push(label.clone());
        args.push(String::new());
        args.push(format!("switch-client -t '{target}'"));
    }
    let _ = Command::new("tmux").args(&args).output()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pane_targets_empty() {
        let map = parse_pane_targets("");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_pane_targets_single_pane() {
        let stdout = "12345\tmain:0.0\tmain\n";
        let map = parse_pane_targets(stdout);
        assert_eq!(map.len(), 1);
        let pane = map.get(&12345).expect("pane");
        assert_eq!(pane.session_name, "main");
        assert_eq!(pane.target, "main:0.0");
    }

    #[test]
    fn parse_pane_targets_multiple_panes() {
        let stdout = "100\twork:0.0\twork\n200\twork:0.1\twork\n300\tdev:1.0\tdev\n";
        let map = parse_pane_targets(stdout);
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(&100).expect("pane").target, "work:0.0");
        assert_eq!(map.get(&200).expect("pane").target, "work:0.1");
        assert_eq!(map.get(&300).expect("pane").session_name, "dev");
    }

    #[test]
    fn parse_pane_targets_invalid_pid_skipped() {
        let stdout = "notapid\tmain:0.0\tmain\n12345\twork:0.0\twork\n";
        let map = parse_pane_targets(stdout);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&12345));
    }

    #[test]
    fn parse_pane_targets_incomplete_line_skipped() {
        let stdout = "12345\tmain:0.0\n67890\twork:0.0\twork\n";
        let map = parse_pane_targets(stdout);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&67890));
    }

    #[test]
    fn parse_pane_targets_duplicate_pid_last_wins() {
        let stdout = "100\tfirst:0.0\tfirst\n100\tsecond:1.0\tsecond\n";
        let map = parse_pane_targets(stdout);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&100).expect("pane").session_name, "second");
    }

    #[test]
    fn parse_sessions_empty() {
        let sessions = parse_sessions("");
        assert!(sessions.is_empty());
    }

    #[test]
    fn parse_sessions_multiple() {
        let stdout = "main\nwork\ndev\n";
        let sessions = parse_sessions(stdout);
        assert_eq!(sessions, vec!["main", "work", "dev"]);
    }

    #[test]
    fn parse_sessions_no_trailing_newline() {
        let stdout = "main\nwork";
        let sessions = parse_sessions(stdout);
        assert_eq!(sessions, vec!["main", "work"]);
    }

    #[test]
    fn parse_pane_targets_session_name_with_spaces() {
        let stdout = "555\tmy session:0.0\tmy session\n";
        let map = parse_pane_targets(stdout);
        assert_eq!(map.len(), 1);
        let pane = map.get(&555).expect("pane");
        assert_eq!(pane.session_name, "my session");
        assert_eq!(pane.target, "my session:0.0");
    }

    #[test]
    fn parse_pane_targets_session_name_with_special_chars() {
        let stdout = "777\tdev-2.0:1.3\tdev-2.0\n";
        let map = parse_pane_targets(stdout);
        assert_eq!(map.len(), 1);
        let pane = map.get(&777).expect("pane");
        assert_eq!(pane.session_name, "dev-2.0");
        assert_eq!(pane.target, "dev-2.0:1.3");
    }

    #[test]
    fn parse_sessions_single() {
        let sessions = parse_sessions("main\n");
        assert_eq!(sessions, vec!["main"]);
    }
}
