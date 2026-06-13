//! co-pipeline CLI — runs the CO-88 pipeline matrix and writes a report.
//!
//! ```text
//! co-pipeline run \
//!   --corpus-root ~/projects \
//!   --paths local,local-api,uat \
//!   --uat-base https://co-artelonga-uat.fly.dev \
//!   --out dev/reports
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use co_pipeline::matrix::{MatrixConfig, RemoteBases};
use co_pipeline::report::PipelineReport;
use co_pipeline::transport::PipePath;

#[derive(Parser)]
#[command(name = "co-pipeline", about = "CO-88 end-to-end pipeline UAT runner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the path × universe × combo matrix and write a report.
    Run(RunArgs),
    /// Print per-universe deltas between two reports (CO-88c).
    Delta(DeltaArgs),
    /// Pre-prod-deploy gate: green, fresh, regressions within tolerance (CO-88d).
    Gate(GateArgs),
}

#[derive(Parser)]
struct DeltaArgs {
    /// Current report YAML.
    #[arg(long)]
    current: PathBuf,
    /// Previous report YAML.
    #[arg(long)]
    previous: PathBuf,
}

#[derive(Parser)]
struct GateArgs {
    /// Report YAML to gate on.
    #[arg(long)]
    report: PathBuf,
    /// Optional baseline report for regression checks.
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Maximum report age in hours.
    #[arg(long, default_value_t = 24)]
    max_age_hours: i64,
    /// Maximum tolerated regression percentage.
    #[arg(long, default_value_t = 20.0)]
    max_regression_pct: f64,
}

#[derive(Parser)]
struct RunArgs {
    /// Root holding the content repos (ArteLonga, quilomboaraucaria, rfq-gateway).
    #[arg(long, default_value = "..")]
    corpus_root: PathBuf,

    /// Comma-separated paths to exercise: local,local-api,uat,prod.
    #[arg(long, default_value = "local")]
    paths: String,

    /// Scratch directory for the `local` filesystem path.
    #[arg(long)]
    scratch: Option<PathBuf>,

    /// Output directory for the report YAML.
    #[arg(long, default_value = "dev/reports")]
    out: PathBuf,

    /// Max files per universe (keeps the run under 60s).
    #[arg(long, default_value_t = 300)]
    file_limit: usize,

    /// Base URL for the local-api path.
    #[arg(long)]
    local_api_base: Option<String>,

    /// Base URL for the UAT path.
    #[arg(long)]
    uat_base: Option<String>,

    /// Base URL for the prod (read-only) path.
    #[arg(long)]
    prod_base: Option<String>,

    /// Bearer token for authenticated vault writes.
    #[arg(long, env = "CO_PIPELINE_TOKEN")]
    token: Option<String>,

    /// Report date stamp (YYYY-MM-DD); defaults to today (UTC).
    #[arg(long)]
    date: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => run(args),
        Command::Delta(args) => delta(args),
        Command::Gate(args) => gate(args),
    }
}

fn delta(args: DeltaArgs) -> ExitCode {
    let current = match co_pipeline::gate::load_report(&args.current) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let previous = match co_pipeline::gate::load_report(&args.previous) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let deltas = co_pipeline::gate::compute_deltas(&current, &previous);
    if deltas.is_empty() {
        println!("No comparable universes between runs.");
    }
    for d in &deltas {
        println!("{}", d.describe());
    }
    ExitCode::SUCCESS
}

fn gate(args: GateArgs) -> ExitCode {
    let report = match co_pipeline::gate::load_report(&args.report) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let baseline = match &args.baseline {
        Some(p) => match co_pipeline::gate::load_report(p) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("warning: ignoring unreadable baseline: {e:#}");
                None
            }
        },
        None => None,
    };
    let config = co_pipeline::gate::GateConfig {
        max_age_hours: args.max_age_hours,
        max_regression_pct: args.max_regression_pct,
    };
    let outcome =
        co_pipeline::gate::evaluate_gate(&report, baseline.as_ref(), chrono::Utc::now(), &config);
    if outcome.passed {
        println!("GATE: pass");
        ExitCode::SUCCESS
    } else {
        for reason in &outcome.reasons {
            eprintln!("GATE FAIL: {reason}");
        }
        ExitCode::FAILURE
    }
}

fn run(args: RunArgs) -> ExitCode {
    let paths = match parse_paths(&args.paths) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let scratch_root = args
        .scratch
        .unwrap_or_else(|| std::env::temp_dir().join("co-pipeline-scratch"));
    // Fresh scratch each run so stale bytes never mask a regression.
    let _ = std::fs::remove_dir_all(&scratch_root);

    let config = MatrixConfig {
        corpus_root: args.corpus_root,
        scratch_root,
        paths: paths.clone(),
        file_limit: args.file_limit,
        remote: RemoteBases {
            local_api: args.local_api_base,
            uat: args.uat_base,
            prod: args.prod_base,
            token: args.token,
        },
    };

    let started = std::time::Instant::now();
    let outcome = co_pipeline::matrix::run(&config);
    let elapsed = started.elapsed();

    let date = args
        .date
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let generated_at = chrono::Utc::now().to_rfc3339();

    let report = PipelineReport::from_cells(
        generated_at,
        env!("CARGO_PKG_VERSION").to_string(),
        &paths,
        &outcome.universes,
        &outcome.cells,
        outcome.errors.clone(),
    );

    if let Err(e) = std::fs::create_dir_all(&args.out) {
        eprintln!("error: cannot create {}: {e}", args.out.display());
        return ExitCode::FAILURE;
    }
    let out_path = args.out.join(format!("co-pipeline-report-{date}.yaml"));
    if let Err(e) = std::fs::write(&out_path, report.to_yaml()) {
        eprintln!("error: cannot write {}: {e}", out_path.display());
        return ExitCode::FAILURE;
    }

    println!(
        "co-pipeline: {} cells, {} files, {} errors in {:.1}s",
        outcome.cells.len(),
        report.global.total_files,
        report.errors.len(),
        elapsed.as_secs_f64(),
    );
    println!("report: {}", out_path.display());

    if report.is_green() {
        println!("RESULT: green");
        ExitCode::SUCCESS
    } else {
        for e in &report.errors {
            eprintln!("FAIL {}/{}/{}: {}", e.path, e.universe, e.combo, e.detail);
        }
        println!("RESULT: red ({} failing cells)", report.errors.len());
        ExitCode::FAILURE
    }
}

fn parse_paths(s: &str) -> Result<Vec<PipePath>, String> {
    s.split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| PipePath::from_label(p).ok_or_else(|| format!("unknown path: {p}")))
        .collect()
}
