use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Monitor Claude Code sessions in tmux")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Update tmux session variables with Claude Code status
    Update {
        /// Filter: all, has-claude, active, idle
        #[arg(default_value = "all")]
        filter: String,
    },
    /// List Claude Code sessions (tab-separated)
    List {
        /// Sort order: timestamp-desc, timestamp-asc, status, status-rev, mode, mode-rev
        #[arg(long)]
        sort: Option<String>,
    },
    /// Open tmux choose-tree with Claude Code status
    Select {
        /// Filter: all, has-claude, active, idle
        #[arg(default_value = "all")]
        filter: String,
    },
    /// Open a Claude-only session picker (fzf or tmux menu)
    Pick {
        /// Sort order: timestamp-desc, timestamp-asc, status, status-rev, mode, mode-rev
        #[arg(long)]
        sort: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Update { filter } => clux::run_update(filter),
        Command::List { sort } => clux::run_list(sort.as_deref()),
        Command::Select { filter } => clux::run_select(filter),
        Command::Pick { sort } => clux::run_pick(sort.as_deref()),
    }
}
