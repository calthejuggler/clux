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

fn run_update() -> Result<(), Box<dyn std::error::Error>> {
    let sessions = claude::discover_sessions();
    let pane_map = tmux::list_panes()?;
    let all_tmux_sessions = tmux::list_sessions()?;

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
            Some(c) => {
                let total = c.active + c.idle;
                let detail = match (c.active, c.idle) {
                    (a, 0) => format!("{a} active"),
                    (0, i) => format!("{i} idle"),
                    (a, i) => format!("{a} active, {i} idle"),
                };
                let info = format!(" | \u{1F916} {total} ({detail})");
                tmux::set_session_option(tmux_session, "@clux_info", &info)?;
            }
            None => {
                tmux::unset_session_option(tmux_session, "@clux_info")?;
            }
        }
    }

    Ok(())
}
