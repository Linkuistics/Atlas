//! CLI plumbing for the four Phase 3 reports — `atlas drift`,
//! `atlas impact`, `atlas modularity`, `atlas divergence`.
//!
//! PR-7 ships only the clap argument structs and stub handlers that
//! print `"<subcommand> is not yet implemented"` to stderr and exit 1.
//! PR-8..PR-11 replace each handler body with the real
//! load-database → call-`atlas_reports::*` → render-or-persist flow.

use std::process::ExitCode;

use anyhow::Result;
use clap::ValueEnum;

/// Output format selected by `--json | --yaml | --human`. Default is
/// `Yaml`. Routing into per-format renderers is wired in PR-8..PR-11;
/// PR-7 only parses the flag so the surface is locked in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// YAML output (default).
    Yaml,
    /// Pretty-printed JSON output.
    Json,
    /// Human-readable indented text output.
    Human,
}

/// `atlas drift` — diff current contract shas against the prior
/// snapshot.
#[derive(Debug, clap::Args)]
pub struct DriftArgs {
    /// Output format. Defaults to `yaml`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Yaml)]
    pub format: OutputFormat,

    /// Compute the drift report but do not write
    /// `.atlas/cache/reports/drift.yaml` or advance the snapshot.
    #[arg(long)]
    pub no_write: bool,
}

/// `atlas impact <id>` — walk downstream consumers of a contract or
/// component. Never persists a file; the `--no-write` flag is
/// intentionally absent here so that `atlas impact --no-write foo`
/// produces a clap-emitted error.
#[derive(Debug, clap::Args)]
pub struct ImpactArgs {
    /// Contract id or component id whose downstream impact set to
    /// compute. The two namespaces are disjoint by Phase 1
    /// construction.
    pub id: String,

    /// Output format. Defaults to `yaml`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Yaml)]
    pub format: OutputFormat,
}

/// `atlas modularity` — per-component metric files plus a top-level
/// rollup.
#[derive(Debug, clap::Args)]
pub struct ModularityArgs {
    /// Output format. Defaults to `yaml`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Yaml)]
    pub format: OutputFormat,

    /// Compute metrics but do not write per-component
    /// `modularity.yaml` files or the rollup.
    #[arg(long)]
    pub no_write: bool,
}

/// `atlas divergence` — pair-wise build-vs-deploy coupling check.
#[derive(Debug, clap::Args)]
pub struct DivergenceArgs {
    /// Output format. Defaults to `yaml`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Yaml)]
    pub format: OutputFormat,

    /// Compute the divergence report but do not write
    /// `.atlas/cache/reports/composition-divergence.yaml`.
    #[arg(long)]
    pub no_write: bool,
}

/// PR-7 stub for `atlas drift`. PR-8 will replace this with the real
/// flow: build engine inputs, call [`atlas_reports::drift`], render,
/// and (unless `--no-write`) atomically persist the report and the
/// snapshot.
pub fn run_drift_cmd(_args: DriftArgs) -> Result<ExitCode> {
    eprintln!("drift is not yet implemented");
    Ok(ExitCode::from(1))
}

/// PR-7 stub for `atlas impact`. PR-9 will replace this with the real
/// flow: build engine inputs, resolve `<id>` into an
/// [`atlas_reports::ImpactTarget`], call [`atlas_reports::impact`],
/// and render to stdout.
pub fn run_impact_cmd(_args: ImpactArgs) -> Result<ExitCode> {
    eprintln!("impact is not yet implemented");
    Ok(ExitCode::from(1))
}

/// PR-7 stub for `atlas modularity`. PR-10 will replace this with the
/// real flow: read each component's prior `modularity.yaml`, call
/// [`atlas_reports::modularity`], rotate history, and (unless
/// `--no-write`) atomically persist the per-component files and
/// rollup.
pub fn run_modularity_cmd(_args: ModularityArgs) -> Result<ExitCode> {
    eprintln!("modularity is not yet implemented");
    Ok(ExitCode::from(1))
}

/// PR-7 stub for `atlas divergence`. PR-11 will replace this with the
/// real flow: read the drift snapshot if any, call
/// [`atlas_reports::divergence`], and (unless `--no-write`) atomically
/// persist the report.
pub fn run_divergence_cmd(_args: DivergenceArgs) -> Result<ExitCode> {
    eprintln!("divergence is not yet implemented");
    Ok(ExitCode::from(1))
}
