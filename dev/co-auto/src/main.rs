//! co-auto — CLI entry point.
//!
//! Single-path API: `--workdir` is the repo root (defaults to CWD). Tasks are
//! discovered by convention at `<workdir>/work/<space>/` (or legacy
//! `<workdir>/data/<space>/`). When the workdir contains exactly one space,
//! `--space` is auto-detected.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use colored::Colorize;

#[derive(Parser)]
#[command(
    name = "co-auto",
    version,
    about = "Automated task execution pipeline (developer tool, separate from the CO scaffold)"
)]
struct Cli {
    /// Repository root containing `work/<space>/` task specs.
    /// Defaults to current working directory.
    #[arg(short, long, env = "CO_WORKDIR")]
    workdir: Option<String>,

    /// Space name (subdir of `work/`). Auto-detected when only one space
    /// exists under `<workdir>/work/`.
    #[arg(short, long, env = "CO_SPACE")]
    space: Option<String>,

    /// Execute a specific task by flag (e.g., `--task CO-66`). See also
    /// the positional `task_arg` for the short form.
    #[arg(short, long)]
    task: Option<String>,

    /// Cycle through tasks continuously.
    #[arg(long)]
    cycle: bool,

    /// Dry run (show what would execute without running).
    #[arg(long)]
    dry_run: bool,

    /// Maximum tasks to process.
    #[arg(long)]
    max_tasks: Option<usize>,

    /// Enable Claude Code agent teams for parallel execution.
    #[arg(long)]
    teams: bool,

    /// Model to use.
    #[arg(long, default_value = "sonnet")]
    model: String,

    /// Timeout per task in seconds (default: 1800 = 30 min).
    #[arg(long, default_value = "1800")]
    timeout: u64,

    /// Run headless (invisible -p mode instead of interactive session).
    #[arg(long)]
    headless: bool,

    /// Push branch + open PR after each successful task (on by default).
    /// Passing this flag is a no-op; use `--no-pr` to disable.
    #[arg(long, default_value_t = true)]
    auto_pr: bool,

    /// Skip the automatic PR step.
    #[arg(long)]
    no_pr: bool,

    /// Task to execute: bare number (`272`) or full key (`CO-272`).
    /// Expanded to a full key using the space's prefix (`co` → `CO-272`).
    /// When omitted, the next unblocked task is picked automatically.
    task_arg: Option<String>,
}

/// Resolve the workdir: explicit arg, env var, or current directory.
fn resolve_workdir(arg: Option<&str>) -> Result<PathBuf> {
    let raw = match arg {
        Some(w) => PathBuf::from(w),
        None => std::env::current_dir().context("read current directory")?,
    };
    if raw.exists() {
        Ok(raw.canonicalize().unwrap_or(raw))
    } else if let Some(home) = dirs::home_dir() {
        let projects = home.join("projects").join(arg.unwrap_or(""));
        if projects.exists() {
            Ok(projects)
        } else {
            Err(anyhow!("workdir not found: {}", raw.display()))
        }
    } else {
        Err(anyhow!("workdir not found: {}", raw.display()))
    }
}

/// Resolve the space + data_dir under the given workdir.
///
/// Looks for `<workdir>/work/<space>/` first (preferred convention), then
/// `<workdir>/data/<space>/` (legacy). When `space` is None, auto-detects:
/// returns the single space if exactly one exists, errors otherwise.
fn resolve_space(workdir: &Path, space: Option<&str>) -> Result<(String, PathBuf)> {
    let candidates: [&str; 2] = ["work", "data"];

    if let Some(s) = space {
        for parent in &candidates {
            let dir = workdir.join(parent).join(s);
            if dir.is_dir() {
                return Ok((s.to_string(), dir));
            }
        }
        return Err(anyhow!(
            "space '{s}' not found under {}/work/ or {}/data/",
            workdir.display(),
            workdir.display()
        ));
    }

    // Auto-detect: scan work/* and data/* for project.yaml
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for parent in &candidates {
        let parent_dir = workdir.join(parent);
        if !parent_dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&parent_dir)
            .with_context(|| format!("read {}", parent_dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir()
                && path.join("project.yaml").exists()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                found.push((name.to_string(), path));
            }
        }
    }

    match found.len() {
        0 => Err(anyhow!(
            "no spaces found under {}/work/ or {}/data/ (looked for subdirs containing project.yaml)",
            workdir.display(),
            workdir.display()
        )),
        1 => Ok(found.into_iter().next().unwrap()),
        n => {
            let names: Vec<String> = found.into_iter().map(|(n, _)| n).collect();
            Err(anyhow!(
                "{n} spaces found ({}), pass --space to disambiguate",
                names.join(", ")
            ))
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let result = (|| -> Result<()> {
        let workdir = resolve_workdir(cli.workdir.as_deref())?;
        let (space, data_dir) = resolve_space(&workdir, cli.space.as_deref())?;

        // Resolve task: positional arg takes priority over --task flag.
        // Bare numbers (e.g., "272") are expanded to full keys ("CO-272")
        // using the space's prefix.
        let task_id = cli
            .task_arg
            .as_deref()
            .map(|arg| co_auto::resolve_task_id(arg, &space, &workdir))
            .or_else(|| cli.task.clone());

        let config = co_auto::AutoConfig {
            space,
            task_id,
            cycle: cli.cycle,
            dry_run: cli.dry_run,
            max_tasks: cli.max_tasks,
            teams: cli.teams,
            model: cli.model,
            timeout_secs: cli.timeout,
            workdir: Some(workdir.to_string_lossy().into_owned()),
            data_dir: Some(data_dir.to_string_lossy().into_owned()),
            workspace: None,
            interactive: !cli.headless,
            auto_pr: cli.auto_pr && !cli.no_pr,
        };

        co_auto::run(config)
    })();

    if let Err(e) = result {
        eprintln!("{}: {e:#}", "error".red().bold());
        std::process::exit(1);
    }
}
