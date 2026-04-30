//! `atlas` — the command-line entry point. Parses arguments with
//! clap, builds the production backend stack (ClaudeCode + Budget +
//! Sentinel), and hands off to [`atlas_cli::run_index`].

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use atlas_cli::{run_index, IndexConfig, IndexError};
use atlas_llm::claude_code::resolve_default_model_id;
use clap::{Parser, Subcommand};

/// Version string baked in at compile time by `build.rs`. Shape:
/// `0.1.0 (v0.1.0-2-g15c2c8c-dirty, built 2026-04-21T06:42:18Z)`.
/// When no tag or no git data is available, the describe slot falls
/// back to the short SHA or literal `unknown`; the timestamp slot
/// falls back to `unknown` only if `date` is unavailable on the
/// build host.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_DESCRIBE"),
    ", built ",
    env!("BUILD_TIMESTAMP"),
    ")"
);

/// Atlas — design recovery for large codebases.
#[derive(Debug, Parser)]
#[command(name = "atlas", version = VERSION, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Index a codebase into the four Atlas YAMLs.
    Index(IndexArgs),
}

#[derive(Debug, clap::Args)]
struct IndexArgs {
    /// Root of the codebase to index.
    root: PathBuf,

    /// Where to write the four Atlas YAMLs. Defaults to
    /// `<root>/.atlas/`.
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// LLM token budget for this run. Fail-loud per §7.4: required
    /// unless `--no-budget` is passed.
    #[arg(long)]
    budget: Option<u64>,

    /// Skip the budget check. Intended for local development only.
    #[arg(long, conflicts_with = "budget")]
    no_budget: bool,

    /// Maximum depth for L8's sub-carve recursion. 0 = top-level
    /// components only.
    #[arg(long, default_value_t = atlas_engine::DEFAULT_MAX_DEPTH)]
    max_depth: u32,

    /// Force L4 to reconsider boundaries — discards prior
    /// `components.yaml` so rename-match does not anchor allocations
    /// to stale ids.
    #[arg(long)]
    recarve: bool,

    /// Compute outputs but do not write them.
    #[arg(long)]
    dry_run: bool,

    /// Override the model id passed to `claude -p --model`. Defaults
    /// to the value of `$ATLAS_LLM_MODEL` or the built-in constant.
    #[arg(long)]
    model: Option<String>,

    /// Disable `.gitignore`-aware filtering when seeding the
    /// filesystem. Useful for tests and for rooting Atlas at a
    /// standalone project that has no `.git` directory.
    #[arg(long)]
    no_gitignore: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("atlas: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Command::Index(args) => run_index_cmd(args),
    }
}

fn run_index_cmd(args: IndexArgs) -> Result<ExitCode> {
    if args.budget.is_none() && !args.no_budget {
        anyhow::bail!(
            "`atlas index` requires `--budget <N-tokens>` to fail loudly on runaway LLM usage. \
             Pass `--no-budget` for local development if you understand the risk."
        );
    }

    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;

    let output_dir = args
        .output_dir
        .unwrap_or_else(|| root.join(atlas_cli::DEFAULT_OUTPUT_SUBDIR));

    let mut config = IndexConfig::new(root);
    config.output_dir = output_dir;
    config.max_depth = args.max_depth;
    config.recarve = args.recarve;
    config.dry_run = args.dry_run;
    config.respect_gitignore = !args.no_gitignore;
    config.prompt_shas = Some(atlas_cli::backend::compute_prompt_shas());

    let model_id = args.model.unwrap_or_else(resolve_default_model_id);
    let handles = atlas_cli::backend::build_production_backend(model_id, args.budget)
        .context("failed to build LLM backend")?;
    config.fingerprint_override = Some(handles.fingerprint.clone());

    let outcome = run_index(&config, handles.backend.clone(), handles.counter.clone());
    match outcome {
        Ok(summary) => {
            println!("{}", atlas_cli::pipeline::format_summary(&summary));
            drop(handles);
            Ok(ExitCode::SUCCESS)
        }
        Err(IndexError::BudgetExhausted) => {
            eprintln!("atlas: LLM token budget exhausted; no output files were written");
            drop(handles);
            Ok(ExitCode::from(2))
        }
        Err(IndexError::Other(err)) => {
            drop(handles);
            Err(err)
        }
    }
}
