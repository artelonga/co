//! co-auto — CLI entry point.
//!
//! Mirrors the flags of the former `co auto` subcommand (CO-84).

use clap::Parser;
use colored::Colorize;

#[derive(Parser)]
#[command(
    name = "co-auto",
    version,
    about = "Automated task execution pipeline (developer tool, separate from the CO scaffold)"
)]
struct Cli {
    /// Target space containing tasks
    #[arg(short, long, default_value = "gp")]
    space: String,

    /// Execute a specific task (e.g., GP-2)
    #[arg(short, long)]
    task: Option<String>,

    /// Cycle through tasks continuously
    #[arg(long)]
    cycle: bool,

    /// Dry run (show what would execute without running)
    #[arg(long)]
    dry_run: bool,

    /// Maximum tasks to process
    #[arg(long)]
    max_tasks: Option<usize>,

    /// Enable Claude Code agent teams for parallel execution
    #[arg(long)]
    teams: bool,

    /// Model to use (default: sonnet)
    #[arg(long, default_value = "sonnet")]
    model: String,

    /// Timeout per task in seconds
    #[arg(long, default_value = "600")]
    timeout: u64,

    /// Working directory for the executor
    #[arg(short, long)]
    workdir: Option<String>,

    /// Explicit data directory (overrides workspace detection)
    #[arg(long, env = "CO_DATA_DIR")]
    data_dir: Option<String>,

    /// CO workspace root (alternative to --data-dir)
    #[arg(long, env = "CO_WORKSPACE")]
    workspace: Option<String>,

    /// Run headless (invisible -p mode instead of interactive session)
    #[arg(long)]
    headless: bool,
}

fn main() {
    let cli = Cli::parse();

    let config = co_auto::AutoConfig {
        space: cli.space,
        task_id: cli.task,
        cycle: cli.cycle,
        dry_run: cli.dry_run,
        max_tasks: cli.max_tasks,
        teams: cli.teams,
        model: cli.model,
        timeout_secs: cli.timeout,
        workdir: cli.workdir,
        data_dir: cli.data_dir,
        workspace: cli.workspace,
        interactive: !cli.headless,
    };

    if let Err(e) = co_auto::run(config) {
        eprintln!("{}: {}", "error".red().bold(), e);
        std::process::exit(1);
    }
}
