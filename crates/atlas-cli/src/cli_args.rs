//! CLI argument structs and the field-translation helpers `main.rs`
//! and contract tests share. Kept in the library half so integration
//! tests can drive the same clap parser + `IndexArgs` → `IndexConfig`
//! mapping the binary uses, without duplicating the wiring (or
//! sidestepping it with substring sniffs on `&[&str]`).

use std::path::PathBuf;

use crate::{IndexConfig, IndexError};

/// `atlas index` arguments. Mirrors the clap-Args definition that used
/// to live in `main.rs`; exposed here so the
/// `strict_overrides_contract.rs` integration test drives the same
/// clap parser and the same field-translation path the binary uses.
#[derive(Debug, clap::Args, Clone)]
pub struct IndexArgs {
    /// Path to the workspace root. Defaults to the current directory.
    pub root: PathBuf,

    /// Where to write the four Atlas YAMLs. Defaults to
    /// `<root>/.atlas/`.
    #[arg(long)]
    pub output_dir: Option<PathBuf>,

    /// LLM token budget for this run. Fail-loud per §7.4: required
    /// unless `--no-budget` is passed.
    #[arg(long)]
    pub budget: Option<u64>,

    /// Skip the budget check. Intended for local development only.
    #[arg(long, conflicts_with = "budget")]
    pub no_budget: bool,

    /// Maximum depth for L8's sub-carve recursion. 0 = top-level
    /// components only.
    #[arg(long, default_value_t = atlas_engine::DEFAULT_MAX_DEPTH)]
    pub max_depth: u32,

    /// Bound on parallel `is_component` calls in L8's map step.
    /// 1 = serial. Defaults to `atlas_engine::DEFAULT_MAP_CONCURRENCY`
    /// (8). Tune lower against rate-limited HTTP providers; higher when
    /// the configured backend is local or unrate-limited.
    #[arg(long, default_value_t = atlas_engine::DEFAULT_MAP_CONCURRENCY)]
    pub map_concurrency: usize,

    /// Force L4 to reconsider boundaries — discards prior
    /// `components.yaml` so rename-match does not anchor allocations
    /// to stale ids.
    #[arg(long)]
    pub recarve: bool,

    /// Compute outputs but do not write them.
    #[arg(long)]
    pub dry_run: bool,

    /// Disable `.gitignore`-aware filtering when seeding the
    /// filesystem. Useful for tests and for rooting Atlas at a
    /// standalone project that has no `.git` directory.
    #[arg(long)]
    pub no_gitignore: bool,

    /// Skip loading `components.overrides.yaml` and
    /// `subsystems.overrides.yaml` from the output directory. The
    /// files on disk are untouched. Forces every candidate through
    /// Atlas's full classification path; useful for cross-target
    /// validation where pin coverage would otherwise short-circuit
    /// the pipeline.
    #[arg(long)]
    pub no_overrides: bool,

    /// Force the per-call progress tally on stderr even when stderr
    /// is not a TTY (e.g., piped to a file). Default behaviour is to
    /// auto-enable when stderr is a TTY.
    #[arg(long, conflicts_with = "no_progress")]
    pub progress: bool,

    /// Suppress the per-call progress tally even when stderr is a
    /// TTY. The final summary line on stdout is unaffected.
    #[arg(long)]
    pub no_progress: bool,

    /// Escalate override warnings (edges_suppress no-match,
    /// edges_add unknown-kind, subsystems.overrides.yaml non-existent
    /// member) to errors. Outputs are still written; the run exits
    /// with a non-zero code if any closed-enumeration override
    /// warning fired.
    #[arg(long)]
    pub strict_overrides: bool,

    /// Disable the TUI subscriber for the LLM-spine runtime (Phase 7);
    /// route events to stdout as JSON-Lines. The TUI subscriber itself
    /// lands in PR-6; PR-2 ships the flag plumbing and the JSON-Lines
    /// fallback. Implied when stdout is not a terminal.
    #[arg(long)]
    pub no_tui: bool,

    /// In addition to TUI or stdout JSON-Lines, log every LLM-spine
    /// event to this file as JSON-Lines (one event per line). Active
    /// in parallel with the other subscribers — useful for post-hoc
    /// analysis without disrupting the user-facing surface. PR-4+
    /// consumes the flag; PR-2 ships the plumbing.
    #[arg(long, value_name = "PATH")]
    pub log_events: Option<PathBuf>,
}

impl IndexArgs {
    /// Apply the parsed args to an existing [`IndexConfig`]. `root` and
    /// `output_dir` are NOT touched (those are pre-canonicalised by the
    /// caller — `main.rs` does the canonicalisation under
    /// `std::fs::canonicalize`; tests pass a pre-built `IndexConfig`
    /// with the tempdir root in place).
    ///
    /// This is the single source of truth for the field mapping —
    /// production code in `run_index_cmd` and the
    /// `strict_overrides_contract.rs` integration test both go through
    /// it so the test cannot drift from the binary's translation.
    pub fn apply_to(&self, config: &mut IndexConfig) {
        config.max_depth = self.max_depth;
        config.map_concurrency = self.map_concurrency;
        config.recarve = self.recarve;
        config.dry_run = self.dry_run;
        config.respect_gitignore = !self.no_gitignore;
        config.no_overrides = self.no_overrides;
        config.strict_overrides = self.strict_overrides;
    }
}

/// Map an [`IndexError`] to the process exit code `main.rs` uses. This
/// is the single source of truth for the exit-code contract —
/// production `run_index_cmd` and the
/// `strict_overrides_contract.rs` integration test both consult it so
/// the test cannot drift from the binary's mapping.
///
/// Codes:
///   `Ok`                              -> 0 (caller's responsibility)
///   `IndexError::StrictOverridesFailed` -> 4
///   `IndexError::BudgetExhausted`       -> 2
///   `IndexError::SetupFailed`           -> 3
///   `IndexError::Other`                 -> 1
pub fn index_error_exit_code(err: &IndexError) -> u8 {
    match err {
        IndexError::StrictOverridesFailed(_) => 4,
        IndexError::BudgetExhausted => 2,
        IndexError::SetupFailed(_) => 3,
        IndexError::Other(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Internal harness — wraps `IndexArgs` in a top-level subcommand
    /// so `try_parse_from` reads the same argv shape `atlas index ...`
    /// the binary parses.
    #[derive(Debug, clap::Parser)]
    #[command(name = "atlas")]
    struct Harness {
        #[command(subcommand)]
        command: HarnessCmd,
    }

    #[derive(Debug, clap::Subcommand)]
    enum HarnessCmd {
        Index(IndexArgs),
    }

    fn parse_index(argv: &[&str]) -> IndexArgs {
        let HarnessCmd::Index(args) = Harness::try_parse_from(argv).unwrap().command;
        args
    }

    #[test]
    fn strict_overrides_default_is_false() {
        let args = parse_index(&["atlas", "index", "/tmp/x"]);
        assert!(!args.strict_overrides);
    }

    #[test]
    fn strict_overrides_flag_binds_to_true() {
        let args = parse_index(&["atlas", "index", "--strict-overrides", "/tmp/x"]);
        assert!(args.strict_overrides);
    }

    #[test]
    fn apply_to_propagates_strict_overrides() {
        let args = parse_index(&["atlas", "index", "--strict-overrides", "/tmp/x"]);
        let mut config = IndexConfig::new(PathBuf::from("/tmp/x"));
        assert!(!config.strict_overrides, "default must be false");
        args.apply_to(&mut config);
        assert!(
            config.strict_overrides,
            "apply_to must set config.strict_overrides from args"
        );
    }

    #[test]
    fn apply_to_leaves_strict_overrides_false_without_flag() {
        let args = parse_index(&["atlas", "index", "/tmp/x"]);
        let mut config = IndexConfig::new(PathBuf::from("/tmp/x"));
        args.apply_to(&mut config);
        assert!(!config.strict_overrides);
    }

    #[test]
    fn no_tui_default_is_false() {
        let args = parse_index(&["atlas", "index", "/tmp/x"]);
        assert!(!args.no_tui);
        assert!(args.log_events.is_none());
    }

    #[test]
    fn no_tui_flag_binds_to_true() {
        let args = parse_index(&["atlas", "index", "--no-tui", "/tmp/x"]);
        assert!(args.no_tui);
    }

    #[test]
    fn log_events_flag_captures_path() {
        let args = parse_index(&[
            "atlas",
            "index",
            "--log-events",
            "/tmp/events.jsonl",
            "/tmp/x",
        ]);
        assert_eq!(
            args.log_events.as_deref(),
            Some(std::path::Path::new("/tmp/events.jsonl"))
        );
    }

    #[test]
    fn exit_code_mapping_is_stable() {
        assert_eq!(
            index_error_exit_code(&IndexError::BudgetExhausted),
            2,
            "budget-exhausted is exit 2"
        );
        assert_eq!(
            index_error_exit_code(&IndexError::SetupFailed("x".into())),
            3,
            "setup-failed is exit 3"
        );
        // StrictOverridesFailed must map to 4 — the load-bearing exit
        // code for `--strict-overrides` regression detection in CI.
        // Constructing an IndexSummary by hand here would couple the
        // unit test to engine-internal types; the production
        // integration test in `strict_overrides_contract.rs` covers
        // the exit-code-4 path end-to-end. Here we cover the other
        // arms.
        assert_eq!(
            index_error_exit_code(&IndexError::Other(anyhow::anyhow!("x"))),
            1,
            "other is exit 1"
        );
    }
}
