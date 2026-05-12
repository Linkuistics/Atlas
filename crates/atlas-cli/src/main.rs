//! `atlas` — the command-line entry point. Parses arguments with
//! clap, builds the production backend stack (ClaudeCode + Budget +
//! Sentinel), and hands off to [`atlas_cli::run_index`].

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use atlas_cli::progress::{make_stderr_reporter, ProgressBackend, ProgressMode};
use atlas_cli::{index_error_exit_code, run_index, IndexArgs, IndexError};
use atlas_llm::LlmBackend;
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
    /// Validate a `components.overrides.yaml` against the canonical
    /// kind vocabulary. Reports unknown kinds, suspicious typos, and
    /// unrecognised pin fields without modifying anything. Exits
    /// non-zero on errors.
    ValidateOverrides(ValidateOverridesArgs),
    /// Scaffold .atlas/ with commented template files before first run.
    Init(InitArgs),
    /// Diff current contract shas against the prior snapshot, listing
    /// changed contracts and any bindings still pinned to the prior
    /// sha. Phase 3, PR-8.
    Drift(reports::DriftArgs),
    /// Walk downstream consumers of a contract or component. Phase 3,
    /// PR-9.
    Impact(reports::ImpactArgs),
    /// Compute per-component modularity metrics (afferent/efferent
    /// coupling, instability, cohesion, surface stability, surface
    /// complexity) plus a subsystem rollup. Phase 3, PR-10.
    Modularity(reports::ModularityArgs),
    /// Flag pairs of components whose build coupling diverges from
    /// their deploy coupling, scored by drift severity against the
    /// snapshot baseline. Phase 3, PR-11.
    Divergence(reports::DivergenceArgs),
}

#[derive(Debug, clap::Args)]
struct ValidateOverridesArgs {
    /// Path to a `components.overrides.yaml` file.
    path: PathBuf,
}

#[derive(Debug, clap::Args)]
struct InitArgs {
    /// Root of the project to initialise. Creates <root>/.atlas/ with
    /// config.yaml, components.overrides.yaml, and subsystems.overrides.yaml.
    root: std::path::PathBuf,
}

use atlas_cli::reports;

mod init;

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
        Command::ValidateOverrides(args) => run_validate_overrides_cmd(args),
        Command::Init(args) => {
            let root = args
                .root
                .canonicalize()
                .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;
            init::run_init_cmd(&root)
        }
        Command::Drift(args) => reports::run_drift_cmd(args),
        Command::Impact(args) => reports::run_impact_cmd(args),
        Command::Modularity(args) => reports::run_modularity_cmd(args),
        Command::Divergence(args) => reports::run_divergence_cmd(args),
    }
}

fn run_validate_overrides_cmd(args: ValidateOverridesArgs) -> Result<ExitCode> {
    let overrides = atlas_index::load_or_default_overrides(&args.path)
        .with_context(|| format!("failed to load {}", args.path.display()))?;
    let subsystems_path = args
        .path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("subsystems.overrides.yaml");
    let subsystems = atlas_index::load_or_default_subsystems_overrides(&subsystems_path)
        .with_context(|| format!("failed to load {}", subsystems_path.display()))?;
    let report = atlas_cli::validate::validate_overrides_with_subsystems(&overrides, &subsystems);
    let mut stdout = std::io::stdout().lock();
    atlas_cli::validate::print_report(&report, &args.path, &mut stdout);
    if report.has_errors() {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_index_cmd(args: IndexArgs) -> Result<ExitCode> {
    // Phase 7 PR-6: short-circuit the entire `atlas index` deterministic
    // pipeline when `--replay-from-cache` is set. The replay path
    // spawns a local tokio runtime (the only legal sync→async
    // boundary, per plan §7.1) and exits without touching the budget,
    // backend, or the deterministic dispatcher. PR-7 will replace the
    // deterministic-dispatcher path below with the
    // `AgentRuntime::run_workspace` path; PR-6 leaves that
    // unmodified.
    if args.replay_from_cache {
        let root = args
            .root
            .canonicalize()
            .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;
        let tui_config = atlas_cli::tui::TuiConfig {
            show_providers: args.tui_show_providers,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build single-thread tokio runtime for --replay-from-cache")?;
        let atlas_root = root.join(atlas_cli::DEFAULT_OUTPUT_SUBDIR);
        // Replay defaults to the ClaudeCode transport. PR-7 surfaces
        // an explicit `--replay-transport` flag if the cross-transport
        // calibration story needs it; PR-6 ships the simplest shape.
        let outcome = runtime.block_on(atlas_cli::replay::replay_into_tui(
            &atlas_root,
            atlas_agents::TransportFlavour::ClaudeCode,
            tui_config,
        ));
        return match outcome {
            Ok(_snapshot) => Ok(ExitCode::SUCCESS),
            Err(err) => {
                eprintln!("atlas: --replay-from-cache failed: {err}");
                Ok(ExitCode::from(1))
            }
        };
    }

    // PR-7: route through the LLM-spine `AgentRuntime` when
    // `--agent-runtime` is set. The single `tokio::block_on` boundary
    // lives inside `run_index_agent_runtime` per plan §7.1 (the only
    // legal sync→async crossover in atlas-cli; the disallowed_methods
    // lint forbids `block_on` in atlas-engine and
    // atlas-agents/src/runtime/). The deterministic-engine path below
    // remains the default — the wiring is the load-bearing PR-7
    // deliverable; production prompts that actually emit ontology-
    // shaped outputs are a follow-up sprint.
    if args.agent_runtime {
        if args.budget.is_none() && !args.no_budget {
            anyhow::bail!(
                "`atlas index --agent-runtime` requires `--budget <N-tokens>` to fail loudly on \
                 runaway LLM usage. Pass `--no-budget` for local development if you understand \
                 the risk."
            );
        }
        let root = args
            .root
            .canonicalize()
            .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;
        let output_dir = args
            .output_dir
            .clone()
            .unwrap_or_else(|| root.join(atlas_cli::DEFAULT_OUTPUT_SUBDIR));
        let config_path = output_dir.join("config.yaml");
        let atlas_config = atlas_llm::AtlasConfig::load(&config_path)
            .with_context(|| format!("failed to load {}", config_path.display()))?;

        let mut index_config = atlas_cli::IndexConfig::new(root);
        index_config.output_dir = output_dir;
        args.apply_to(&mut index_config);
        index_config.prompt_shas = Some(atlas_cli::backend::compute_prompt_shas());

        let counter = args
            .budget
            .map(|b| Arc::new(atlas_llm::TokenCounter::new(b)));

        let handles = atlas_cli::backend::build_production_backend_with_counter(
            &atlas_config,
            &index_config.root,
            counter.clone(),
            None,
        )
        .context("failed to build LLM backend for --agent-runtime")?;

        match atlas_cli::pipeline::run_index_agent_runtime(
            &index_config,
            &atlas_config,
            handles,
            &args,
        ) {
            Ok(()) => return Ok(ExitCode::SUCCESS),
            Err(IndexError::Other(err)) => return Err(err),
            Err(err) => {
                eprintln!("atlas: --agent-runtime failed: {err}");
                let code = index_error_exit_code(&err);
                return Ok(ExitCode::from(code));
            }
        }
    }

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
        .clone()
        .unwrap_or_else(|| root.join(atlas_cli::DEFAULT_OUTPUT_SUBDIR));

    let config_path = output_dir.join("config.yaml");
    let atlas_config = atlas_llm::AtlasConfig::load(&config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;

    let mut index_config = atlas_cli::IndexConfig::new(root);
    index_config.output_dir = output_dir;
    // Single source of truth for `IndexArgs` → `IndexConfig`; shared
    // with the strict-overrides contract integration test so the test
    // exercises the same translation the binary uses.
    args.apply_to(&mut index_config);
    index_config.prompt_shas = Some(atlas_cli::backend::compute_prompt_shas());

    let progress_mode = if args.no_progress {
        ProgressMode::Never
    } else if args.progress {
        ProgressMode::Always
    } else {
        ProgressMode::Auto
    };

    // Build the token counter up here so the reporter and the backend
    // share a single instance — otherwise the gauge and the budgeted
    // accounting can diverge.
    let counter = args
        .budget
        .map(|b| Arc::new(atlas_llm::TokenCounter::new(b)));
    let reporter = make_stderr_reporter(progress_mode, counter.clone());

    let observer = if reporter.drawing() {
        Some(Arc::clone(&reporter) as Arc<dyn atlas_llm::BackendCallObserver>)
    } else {
        None
    };

    let handles = atlas_cli::backend::build_production_backend_with_counter(
        &atlas_config,
        &index_config.root,
        counter.clone(),
        observer,
    )
    .context("failed to build LLM backend")?;
    index_config.fingerprint_override = Some(handles.fingerprint.clone());

    let backend: Arc<dyn LlmBackend> =
        ProgressBackend::new(handles.backend.clone(), Arc::clone(&reporter)) as Arc<dyn LlmBackend>;

    let outcome = run_index(
        &index_config,
        backend,
        handles.counter.clone(),
        Arc::clone(&reporter),
    );
    reporter.finish();
    match outcome {
        Ok(summary) => {
            println!("{}", atlas_cli::pipeline::format_summary(&summary));
            drop(handles);
            Ok(ExitCode::SUCCESS)
        }
        Err(IndexError::Other(err)) => {
            drop(handles);
            Err(err)
        }
        Err(err) => {
            // Diagnostic strings — `index_error_exit_code` owns the
            // exit-code contract (shared with
            // `crates/atlas-cli/tests/strict_overrides_contract.rs`).
            match &err {
                IndexError::BudgetExhausted => {
                    eprintln!("atlas: LLM token budget exhausted; no output files were written");
                }
                IndexError::SetupFailed(msg) => {
                    eprintln!(
                        "atlas: LLM backend setup failed: {msg}; no output files were written"
                    );
                }
                IndexError::StrictOverridesFailed(summary) => {
                    // PR-4: outputs were still written; this is a
                    // strict-mode exit-code gate on top of an
                    // otherwise-successful run. The collector already
                    // echoed every offending warning to stderr.
                    println!("{}", atlas_cli::pipeline::format_summary(summary));
                    eprintln!(
                        "atlas: --strict-overrides set; override warnings escalated to errors \
                         (see warnings above)"
                    );
                }
                IndexError::Other(_) => unreachable!("handled above"),
            }
            let code = index_error_exit_code(&err);
            drop(handles);
            Ok(ExitCode::from(code))
        }
    }
}
