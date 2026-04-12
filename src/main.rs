mod claude;
mod history;
mod process;
mod tmux;

use std::collections::HashMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("update") => {
            let filter = args.get(2).map_or("all", String::as_str);
            if let Err(err) = run_update(filter) {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        Some("list") => {
            if let Err(err) = run_list() {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        Some("select") => {
            let filter = args.get(2).map_or("all", String::as_str);
            if let Err(err) = run_select(filter) {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        Some("pick") => {
            if let Err(err) = run_pick() {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Usage: clux <update [filter]|list|select [filter]|pick>");
            std::process::exit(1);
        }
    }
}

struct SessionCounts {
    active: u32,
    idle: u32,
}

struct ListEntry {
    target: String,
    state: &'static str,
    mode: &'static str,
    active_tasks: u32,
    active_agents: u32,
    summary: String,
    cwd: String,
    session_name: String,
}

const DEFAULT_FORMAT: &str = " | \u{1F916} {total} ({detail})";

fn format_info(format_str: &str, counts: &SessionCounts) -> String {
    let total = counts.active + counts.idle;
    let detail = match (counts.active, counts.idle) {
        (active, 0) => format!("{active} active"),
        (0, idle) => format!("{idle} idle"),
        (active, idle) => format!("{active} active, {idle} idle"),
    };

    format_str
        .replace("{total}", &total.to_string())
        .replace("{active}", &counts.active.to_string())
        .replace("{idle}", &counts.idle.to_string())
        .replace("{detail}", &detail)
}

fn is_visible(filter: &str, counts: Option<&SessionCounts>) -> bool {
    match filter {
        "has-claude" => counts.is_some(),
        "active" => counts.is_some_and(|ct| ct.active > 0),
        "idle" => counts.is_some_and(|ct| ct.idle > 0),
        _ => true,
    }
}

fn shorten_cwd(cwd: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if cwd.starts_with(&home) => {
            let mut short = String::from("~");
            short.push_str(cwd.get(home.len()..).unwrap_or_default());
            short
        }
        _ => cwd.to_owned(),
    }
}

fn truncate_at(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_owned();
    }
    let truncated: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{truncated}...")
}

fn command_exists(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|exit_status| exit_status.success())
}

fn gather_list_entries() -> Result<Vec<ListEntry>, Box<dyn std::error::Error>> {
    let sessions = claude::discover_sessions();
    let pane_map = tmux::list_pane_targets()?;
    let proc_tree = process::ProcessTree::build();
    let summaries = history::load_summaries(&sessions);

    let mut with_panes: Vec<_> = sessions
        .iter()
        .filter_map(|sess| {
            process::find_tmux_pane(sess.pid, &pane_map, &proc_tree).map(|pane| (sess, pane))
        })
        .collect();
    with_panes.sort_by(|(session_a, _), (session_b, _)| {
        let ts_a = summaries
            .get(&session_a.pid)
            .map_or(session_a.started_at, |smry| smry.timestamp);
        let ts_b = summaries
            .get(&session_b.pid)
            .map_or(session_b.started_at, |smry| smry.timestamp);
        ts_b.cmp(&ts_a)
    });

    let entries = with_panes
        .iter()
        .map(|(session, pane)| {
            let info = claude::detect_info(session);
            let state_str = match info.state {
                claude::SessionState::Active => "active",
                claude::SessionState::Idle => "idle",
            };
            let mode_str = match info.mode {
                claude::SessionMode::Default => "default",
                claude::SessionMode::AcceptEdits => "acceptEdits",
                claude::SessionMode::BypassPermissions => "yolo",
                claude::SessionMode::Plan => "plan",
            };
            let summary_text = summaries
                .get(&session.pid)
                .map_or("(no summary)", |smry| smry.display.as_str());

            ListEntry {
                target: pane.target.clone(),
                state: state_str,
                mode: mode_str,
                active_tasks: info.active_tasks,
                active_agents: info.active_agents,
                summary: summary_text.to_owned(),
                cwd: shorten_cwd(&session.cwd),
                session_name: pane.session_name.clone(),
            }
        })
        .collect();

    Ok(entries)
}

fn run_list() -> Result<(), Box<dyn std::error::Error>> {
    let entries = gather_list_entries()?;
    for entry in &entries {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            entry.target,
            entry.state,
            entry.mode,
            entry.active_tasks,
            entry.active_agents,
            entry.summary,
            entry.cwd,
            entry.session_name
        );
    }
    Ok(())
}

fn run_update(filter: &str) -> Result<(), Box<dyn std::error::Error>> {
    let sessions = claude::discover_sessions();
    let pane_map = tmux::list_pane_targets()?;
    let proc_tree = process::ProcessTree::build();
    let all_tmux_sessions = tmux::list_sessions()?;
    let custom_format = tmux::get_global_option("@clux-format")?;
    let format_str = custom_format.as_deref().unwrap_or(DEFAULT_FORMAT);

    let mut counts: HashMap<String, SessionCounts> = HashMap::new();

    for session in &sessions {
        if let Some(pane) = process::find_tmux_pane(session.pid, &pane_map, &proc_tree) {
            let info = claude::detect_info(session);
            let entry = counts
                .entry(pane.session_name.clone())
                .or_insert(SessionCounts { active: 0, idle: 0 });
            match info.state {
                claude::SessionState::Active => entry.active += 1,
                claude::SessionState::Idle => entry.idle += 1,
            }
        }
    }

    for tmux_session in &all_tmux_sessions {
        let session_counts = counts.get(tmux_session);
        let visible = is_visible(filter, session_counts);
        tmux::set_session_option(
            tmux_session,
            "@clux_visible",
            if visible { "1" } else { "0" },
        )?;

        match session_counts {
            Some(ct) => {
                let info = format_info(format_str, ct);
                tmux::set_session_option(tmux_session, "@clux_info", &info)?;
            }
            None => {
                tmux::unset_session_option(tmux_session, "@clux_info")?;
            }
        }
    }

    Ok(())
}

fn run_select(filter: &str) -> Result<(), Box<dyn std::error::Error>> {
    run_update(filter)?;
    tmux::choose_tree(filter)?;
    Ok(())
}

fn run_pick() -> Result<(), Box<dyn std::error::Error>> {
    let entries = gather_list_entries()?;
    if entries.is_empty() {
        tmux::display_message("clux: no Claude sessions found")?;
        return Ok(());
    }

    let fzf_option = tmux::get_global_option("@clux-fzf")?;
    let fzf_disabled = fzf_option.as_deref() == Some("off");

    if !fzf_disabled && command_exists("fzf-tmux") {
        pick_with_fzf(&entries)?;
    } else {
        pick_with_menu(&entries)?;
    }

    Ok(())
}

fn pick_with_fzf(entries: &[ListEntry]) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write as _;

    let header = format!(
        "{:<7}  {:<11}  {:>5}  {:>6}  {:<40}  {:<25}  {}",
        "STATE", "MODE", "TASKS", "AGENTS", "SUMMARY", "CWD", "SESSION"
    );

    let rows: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "{}\t{:<7}  {:<11}  {:>5}  {:>6}  {:<40}  {:<25}  {}",
                entry.target,
                entry.state,
                entry.mode,
                entry.active_tasks,
                entry.active_agents,
                truncate_at(&entry.summary, 40),
                truncate_at(&entry.cwd, 25),
                entry.session_name
            )
        })
        .collect();

    let input = rows.join("\n");

    let mut child = std::process::Command::new("fzf-tmux")
        .args([
            "-p",
            "80%,50%",
            "--delimiter",
            "\t",
            "--with-nth",
            "2..",
            "--header",
            &header,
            "--no-preview",
            "--reverse",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;

    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(input.as_bytes())?;
    }

    let fzf_output = child.wait_with_output()?;

    if !fzf_output.status.success() {
        return Ok(());
    }

    let selected = String::from_utf8_lossy(&fzf_output.stdout);
    let trimmed = selected.trim();
    if let Some(chosen_target) = trimmed.split('\t').next()
        && !chosen_target.is_empty()
    {
        tmux::switch_client(chosen_target)?;
    }

    Ok(())
}

fn pick_with_menu(entries: &[ListEntry]) -> Result<(), Box<dyn std::error::Error>> {
    let items: Vec<(String, String)> = entries
        .iter()
        .map(|entry| {
            let label = format!(
                "{} | {} | {} tasks | {} agents | {} | {} ({})",
                entry.state,
                entry.mode,
                entry.active_tasks,
                entry.active_agents,
                entry.summary,
                entry.cwd,
                entry.session_name
            );
            (truncate_at(&label, 70), entry.target.clone())
        })
        .collect();

    tmux::display_menu("Claude Sessions", &items)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_info_all_active() {
        let counts = SessionCounts { active: 3, idle: 0 };
        let result = format_info(DEFAULT_FORMAT, &counts);
        assert!(result.contains("3"));
        assert!(result.contains("3 active"));
    }

    #[test]
    fn format_info_all_idle() {
        let counts = SessionCounts { active: 0, idle: 2 };
        let result = format_info(DEFAULT_FORMAT, &counts);
        assert!(result.contains("2"));
        assert!(result.contains("2 idle"));
    }

    #[test]
    fn format_info_mixed() {
        let counts = SessionCounts { active: 1, idle: 2 };
        let result = format_info(DEFAULT_FORMAT, &counts);
        assert!(result.contains("3"));
        assert!(result.contains("1 active, 2 idle"));
    }

    #[test]
    fn format_info_custom_template() {
        let counts = SessionCounts { active: 2, idle: 3 };
        let result = format_info("T={total} A={active} I={idle} D={detail}", &counts);
        assert_eq!(result, "T=5 A=2 I=3 D=2 active, 3 idle");
    }

    #[test]
    fn is_visible_all_filter_always_true() {
        assert!(is_visible("all", None));
        assert!(is_visible(
            "all",
            Some(&SessionCounts { active: 0, idle: 0 })
        ));
    }

    #[test]
    fn is_visible_has_claude_some() {
        assert!(is_visible(
            "has-claude",
            Some(&SessionCounts { active: 0, idle: 0 })
        ));
    }

    #[test]
    fn is_visible_has_claude_none() {
        assert!(!is_visible("has-claude", None));
    }

    #[test]
    fn is_visible_active_filter() {
        assert!(is_visible(
            "active",
            Some(&SessionCounts { active: 1, idle: 0 })
        ));
        assert!(!is_visible(
            "active",
            Some(&SessionCounts { active: 0, idle: 1 })
        ));
        assert!(!is_visible("active", None));
    }

    #[test]
    fn is_visible_idle_filter() {
        assert!(is_visible(
            "idle",
            Some(&SessionCounts { active: 0, idle: 1 })
        ));
        assert!(!is_visible(
            "idle",
            Some(&SessionCounts { active: 1, idle: 0 })
        ));
        assert!(!is_visible("idle", None));
    }

    #[test]
    fn is_visible_unknown_filter_defaults_true() {
        assert!(is_visible("unknown", None));
    }

    #[test]
    fn shorten_cwd_with_home() {
        let home = std::env::var("HOME").expect("HOME must be set");
        let path = format!("{home}/projects/test");
        assert_eq!(shorten_cwd(&path), "~/projects/test");
    }

    #[test]
    fn shorten_cwd_without_home() {
        assert_eq!(shorten_cwd("/tmp/other"), "/tmp/other");
    }

    #[test]
    fn shorten_cwd_exact_home() {
        let home = std::env::var("HOME").expect("HOME must be set");
        assert_eq!(shorten_cwd(&home), "~");
    }

    #[test]
    fn format_info_zero_counts() {
        let counts = SessionCounts { active: 0, idle: 0 };
        let result = format_info(DEFAULT_FORMAT, &counts);
        assert!(result.contains("0"));
        assert!(result.contains("0 active"));
    }

    #[test]
    fn command_exists_known_good() {
        assert!(command_exists("sh"));
    }

    #[test]
    fn command_exists_known_bad() {
        assert!(!command_exists(
            "this_command_definitely_does_not_exist_xyz"
        ));
    }

    #[test]
    fn truncate_at_short() {
        assert_eq!(truncate_at("hello", 10), "hello");
    }

    #[test]
    fn truncate_at_exact() {
        assert_eq!(truncate_at("hello", 5), "hello");
    }

    #[test]
    fn truncate_at_long() {
        let result = truncate_at("hello world!", 8);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn truncate_at_unicode() {
        let result = truncate_at("hellooo\u{1F916}world", 10);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 10);
    }
}
