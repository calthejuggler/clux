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
    List,
    /// Open tmux choose-tree with Claude Code status
    Select {
        /// Filter: all, has-claude, active, idle
        #[arg(default_value = "all")]
        filter: String,
    },
    /// Open a Claude-only session picker (fzf or tmux menu)
    Pick,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Update { filter } => clux::run_update(filter),
        Command::List => clux::run_list(),
        Command::Select { filter } => clux::run_select(filter),
        Command::Pick => clux::run_pick(),
    }
}
