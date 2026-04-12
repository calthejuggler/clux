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
            if let Err(e) = run_update(filter) {
                eprintln!("clux: {e}");
                std::process::exit(1);
            }
        }
        Some("list") => {
            if let Err(e) = run_list() {
                eprintln!("clux: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Usage: clux <update [filter]|list>");
            std::process::exit(1);
        }
    }
}

struct SessionCounts {
    active: u32,
    idle: u32,
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
        "active" => counts.is_some_and(|c| c.active > 0),
        "idle" => counts.is_some_and(|c| c.idle > 0),
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

fn run_list() -> Result<(), Box<dyn std::error::Error>> {
    let sessions = claude::discover_sessions();
    let pane_map = tmux::list_pane_targets()?;
    let proc_tree = process::ProcessTree::build();
    let summaries = history::load_summaries(&sessions);

    let mut with_panes: Vec<_> = sessions
        .iter()
        .filter_map(|s| process::find_tmux_pane(s.pid, &pane_map, &proc_tree).map(|p| (s, p)))
        .collect();
    with_panes.sort_by(|(session_a, _), (session_b, _)| {
        let ts_a = summaries.get(&session_a.pid).map_or(0, |s| s.timestamp);
        let ts_b = summaries.get(&session_b.pid).map_or(0, |s| s.timestamp);
        ts_b.cmp(&ts_a)
    });

    for (session, pane) in &with_panes {
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

        let summary = summaries
            .get(&session.pid)
            .map_or("(no summary)", |s| s.display.as_str());

        let cwd = shorten_cwd(&session.cwd);

        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            pane.target,
            state_str,
            mode_str,
            info.active_tasks,
            info.active_agents,
            summary,
            cwd,
            pane.session_name
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
            Some(c) => {
                let info = format_info(format_str, c);
                tmux::set_session_option(tmux_session, "@clux_info", &info)?;
            }
            None => {
                tmux::unset_session_option(tmux_session, "@clux_info")?;
            }
        }
    }

    Ok(())
}
