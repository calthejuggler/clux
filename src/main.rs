mod claude;
mod process;
mod tmux;

use std::collections::HashMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("update") {
        if let Err(e) = run_update() {
            eprintln!("clux: {e}");
            std::process::exit(1);
        }
    } else {
        eprintln!("Usage: clux update");
        std::process::exit(1);
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

fn run_update() -> Result<(), Box<dyn std::error::Error>> {
    let sessions = claude::discover_sessions();
    let pane_map = tmux::list_panes()?;
    let all_tmux_sessions = tmux::list_sessions()?;
    let custom_format = tmux::get_global_option("@clux-format")?;
    let format_str = custom_format.as_deref().unwrap_or(DEFAULT_FORMAT);

    let mut counts: HashMap<String, SessionCounts> = HashMap::new();

    for session in &sessions {
        if let Some(tmux_session) = process::find_tmux_session(session.pid, &pane_map) {
            let state = claude::detect_state(session);
            let entry = counts
                .entry(tmux_session)
                .or_insert(SessionCounts { active: 0, idle: 0 });
            match state {
                claude::SessionState::Active => entry.active += 1,
                claude::SessionState::Idle => entry.idle += 1,
            }
        }
    }

    for tmux_session in &all_tmux_sessions {
        match counts.get(tmux_session) {
            Some(session_counts) => {
                let info = format_info(format_str, session_counts);
                tmux::set_session_option(tmux_session, "@clux_info", &info)?;
            }
            None => {
                tmux::unset_session_option(tmux_session, "@clux_info")?;
            }
        }
    }

    Ok(())
}
