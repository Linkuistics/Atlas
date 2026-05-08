//! CLI plumbing for the four Phase 3 reports — `atlas drift`,
//! `atlas impact`, `atlas modularity`, `atlas divergence`.
//!
//! PR-7 ships only the clap argument structs and stub handlers that
//! print `"<subcommand> is not yet implemented"` to stderr and exit 1.
//! PR-8 lands the real `atlas drift` flow:
//!
//! 1. Resolve the workspace primary root (canonicalise the cwd).
//! 2. Read `<root>/.atlas/cache/components.yaml` (the prior `atlas
//!    index` output) and each live component's
//!    `<component>/.atlas/cache/surfaces.yaml`.
//! 3. Lift out every contract's `content_sha` and every
//!    binding-consumes-contract relationship into the flat-collection
//!    inputs [`atlas_reports::drift_pure`] expects.
//! 4. Read the prior snapshot from
//!    `<root>/.atlas/cache/contract-shas-snapshot.yaml` (or `None` on
//!    first run; parse errors degrade to `None` with a warning).
//! 5. Call [`atlas_reports::drift_pure`].
//! 6. Render the report to stdout in the requested format.
//! 7. Unless `--no-write`: atomically write the report to
//!    `<root>/.atlas/cache/reports/drift.yaml` and the new snapshot
//!    to `<root>/.atlas/cache/contract-shas-snapshot.yaml`.
//! 8. Print a one-line summary; first-run UX prints a guidance
//!    message instead.
//!
//! Reading directly from the on-disk YAMLs (rather than recomputing
//! the engine database) matches design §3.1's "reports observe what
//! the engine has already produced" rule and keeps `atlas drift`
//! cheap — no LLM calls, no fixedpoint iteration.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use atlas_engine::atomic_write;
use atlas_index::{
    load_or_default_components, ComponentEntry, ComponentsFile, ContractKind, SurfacesFile,
};
use atlas_reports::{
    drift_pure, ContractShaSnapshot, CurrentBinding, CurrentContract, DriftReport,
    DERIVED_FROM_CONTRACT_SHA_ATTR,
};
use chrono::Utc;
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

    /// Workspace primary root. Defaults to the current working
    /// directory. Canonicalised before use so the resolved
    /// `<root>/.atlas/` paths are always absolute.
    #[arg(long)]
    pub root: Option<PathBuf>,
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

/// Default name for the workspace `.atlas/` directory. Matches
/// `atlas_cli::DEFAULT_OUTPUT_SUBDIR`; copied here as a `&'static
/// str` so the reports module does not pull in pipeline state.
const DEFAULT_OUTPUT_SUBDIR: &str = ".atlas";

/// Filename of the drift snapshot baseline under
/// `<root>/.atlas/cache/`.
const SNAPSHOT_FILENAME: &str = "contract-shas-snapshot.yaml";

/// Relative location of the drift report under `<root>/.atlas/cache/`.
const REPORT_RELPATH: &str = "reports/drift.yaml";

/// Run `atlas drift`. See module docs for the high-level flow.
pub fn run_drift_cmd(args: DriftArgs) -> Result<ExitCode> {
    let mut stdout = std::io::stdout().lock();
    run_drift(&args, &mut stdout)
}

/// Inner implementation, parameterised over the stdout sink for
/// testability. Tests pass a `Vec<u8>` and assert on the rendered
/// bytes.
pub fn run_drift<W: Write>(args: &DriftArgs, stdout: &mut W) -> Result<ExitCode> {
    let root = match &args.root {
        Some(r) => r
            .canonicalize()
            .with_context(|| format!("failed to resolve --root path {}", r.display()))?,
        None => std::env::current_dir()
            .context("failed to read current working directory for `atlas drift`")?
            .canonicalize()
            .context("failed to canonicalise current working directory")?,
    };
    let output_dir = root.join(DEFAULT_OUTPUT_SUBDIR);
    let cache_dir = output_dir.join("cache");
    let snapshot_path = cache_dir.join(SNAPSHOT_FILENAME);
    let report_path = cache_dir.join(REPORT_RELPATH);
    let components_path = cache_dir.join("components.yaml");

    // ---- 1. Load components.yaml --------------------------------------
    if !components_path.exists() {
        anyhow::bail!(
            "no components.yaml found at {}. Run `atlas index` first.",
            components_path.display()
        );
    }
    let components_file: ComponentsFile = load_or_default_components(&components_path)
        .with_context(|| format!("failed to read {}", components_path.display()))?;

    // The persisted roots inside components.yaml are the canonical
    // analysed-root set; fall back to the resolved primary root if
    // they are absent (defensive — `atlas index` always populates
    // `roots`).
    let roots: Vec<PathBuf> = if components_file.roots.is_empty() {
        vec![root.clone()]
    } else {
        components_file.roots.clone()
    };

    // ---- 2. Walk components → contracts + bindings -------------------
    let (current_contracts, current_bindings) =
        collect_current_state_from_disk(&components_file, &roots);

    // ---- 3. Read prior snapshot --------------------------------------
    let prev_snapshot = read_prev_snapshot(&snapshot_path);

    // ---- 4. Compute drift --------------------------------------------
    let now = Utc::now();
    let (report, new_snapshot) = drift_pure(
        &current_contracts,
        &current_bindings,
        prev_snapshot.as_ref(),
        now,
        now,
    );

    // ---- 5. Render to stdout -----------------------------------------
    render_to(stdout, &report, args.format).context("failed to render drift report to stdout")?;

    // ---- 6. Persist (unless --no-write) ------------------------------
    let is_first_run = prev_snapshot.is_none();
    if !args.no_write {
        write_drift_outputs(&report, &new_snapshot, &report_path, &snapshot_path)?;
    }

    // ---- 7. One-line summary ------------------------------------------
    if is_first_run {
        eprintln!(
            "No prior baseline found. Captured baseline of {} contracts. \
             Run `atlas drift` again after changes to see drift.",
            report.summary.total_contracts
        );
    } else {
        eprintln!(
            "drift: {} changed, {} added, {} removed, {} pinned bindings",
            report.summary.changed,
            report.summary.added,
            report.summary.removed,
            report.summary.pinned_bindings_count,
        );
    }

    Ok(ExitCode::SUCCESS)
}

/// Render `report` to `out` in the requested format.
fn render_to<W: Write>(
    out: &mut W,
    report: &DriftReport,
    format: OutputFormat,
) -> std::io::Result<()> {
    match format {
        OutputFormat::Yaml => {
            let yaml = serde_yaml::to_string(report).map_err(std::io::Error::other)?;
            out.write_all(yaml.as_bytes())
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(report)?;
            out.write_all(json.as_bytes())?;
            out.write_all(b"\n")
        }
        OutputFormat::Human => render_human(out, report),
    }
}

/// Indented free-text rendering used by `--format human`. Compact;
/// the YAML form remains the canonical machine-readable output.
fn render_human<W: Write>(out: &mut W, report: &DriftReport) -> std::io::Result<()> {
    writeln!(out, "drift report (generated_at: {})", report.generated_at)?;
    if let Some(baseline) = report.baseline_captured_at {
        writeln!(out, "  baseline_captured_at: {baseline}")?;
    } else {
        writeln!(out, "  baseline_captured_at: <none — first run>")?;
    }
    writeln!(out, "  total_contracts: {}", report.summary.total_contracts)?;
    writeln!(out, "  changed: {}", report.summary.changed)?;
    writeln!(out, "  added: {}", report.summary.added)?;
    writeln!(out, "  removed: {}", report.summary.removed)?;
    writeln!(
        out,
        "  pinned_bindings_count: {}",
        report.summary.pinned_bindings_count
    )?;

    if !report.contracts_changed.is_empty() {
        writeln!(out, "contracts_changed:")?;
        for c in &report.contracts_changed {
            writeln!(
                out,
                "  - {}: {} -> {}",
                c.id, c.prior_content_sha, c.current_content_sha
            )?;
            for p in &c.pinned_bindings {
                writeln!(
                    out,
                    "      pinned binding in {} ({}): {}",
                    p.component.as_str(),
                    p.language,
                    p.binding_content_sha
                )?;
            }
        }
    }
    if !report.contracts_added.is_empty() {
        writeln!(out, "contracts_added:")?;
        for c in &report.contracts_added {
            writeln!(out, "  - {} ({})", c.id, c.current_content_sha)?;
        }
    }
    if !report.contracts_removed.is_empty() {
        writeln!(out, "contracts_removed:")?;
        for c in &report.contracts_removed {
            writeln!(out, "  - {} ({})", c.id, c.prior_content_sha)?;
        }
    }
    Ok(())
}

/// Atomically write the drift report and new snapshot. Both writes
/// use [`atomic_write`] so a kill mid-write leaves the destination
/// fully-old (or absent on first run).
fn write_drift_outputs(
    report: &DriftReport,
    new_snapshot: &ContractShaSnapshot,
    report_path: &Path,
    snapshot_path: &Path,
) -> Result<()> {
    let report_yaml =
        serde_yaml::to_string(report).context("failed to serialise drift report to YAML")?;
    atomic_write(report_path, report_yaml.as_bytes())
        .with_context(|| format!("failed to write {}", report_path.display()))?;

    let snapshot_yaml = serde_yaml::to_string(new_snapshot)
        .context("failed to serialise contract-shas-snapshot to YAML")?;
    atomic_write(snapshot_path, snapshot_yaml.as_bytes())
        .with_context(|| format!("failed to write {}", snapshot_path.display()))?;

    Ok(())
}

/// Read the prior snapshot from disk, returning `None` if the file
/// does not exist or cannot be parsed (the latter degrades to "first
/// run" with a warning so a corrupt baseline does not lock the user
/// out of `atlas drift`).
fn read_prev_snapshot(path: &Path) -> Option<ContractShaSnapshot> {
    if !path.exists() {
        return None;
    }
    match fs::read_to_string(path) {
        Ok(text) => match serde_yaml::from_str::<ContractShaSnapshot>(&text) {
            Ok(snap) => Some(snap),
            Err(err) => {
                eprintln!(
                    "warning: failed to parse {} as a contract-shas snapshot: {err}; \
                     treating this run as first-run (baseline will be re-captured)",
                    path.display()
                );
                None
            }
        },
        Err(err) => {
            eprintln!(
                "warning: failed to read {}: {err}; treating this run as first-run",
                path.display()
            );
            None
        }
    }
}

/// Walk every live component in `components_file`, locate its
/// `<component>/.atlas/cache/surfaces.yaml` on disk, and lift out the
/// contracts and binding-consumes-contract relationships needed by
/// [`drift_pure`].
///
/// Per-component read failures are non-fatal warnings: a missing
/// surfaces.yaml just means that component contributes no contracts /
/// bindings to the drift comparison. The top-level summary still
/// reports across whatever components did parse.
fn collect_current_state_from_disk(
    components_file: &ComponentsFile,
    roots: &[PathBuf],
) -> (Vec<CurrentContract>, Vec<CurrentBinding>) {
    let mut contracts: Vec<CurrentContract> = Vec::new();
    let mut bindings: Vec<CurrentBinding> = Vec::new();

    for entry in &components_file.components {
        if entry.deleted {
            continue;
        }
        let surfaces_path = match resolve_surfaces_path(entry, roots) {
            Some(p) => p,
            None => continue,
        };
        if !surfaces_path.exists() {
            // Surfaces file missing — usually means the component is
            // not yet analysed (e.g. a non-Rust component for which
            // the surfaces emitter does not yet run). Skip silently.
            continue;
        }
        let bytes = match fs::read(&surfaces_path) {
            Ok(b) => b,
            Err(err) => {
                eprintln!(
                    "warning: failed to read {}: {err}; skipping",
                    surfaces_path.display()
                );
                continue;
            }
        };
        let surfaces: SurfacesFile = match serde_yaml::from_slice(&bytes) {
            Ok(s) => s,
            Err(err) => {
                eprintln!(
                    "warning: failed to parse {}: {err}; skipping",
                    surfaces_path.display()
                );
                continue;
            }
        };

        // Every contract the component defines contributes its
        // canonical `content_sha` (stored in the surfaces schema as
        // `Contract::fingerprint` per spec §2 / §4 of the
        // canonicalisation doc).
        for c in &surfaces.contracts_defined {
            // Skip library-API contracts — drift compares schema /
            // data-format contracts only. Library APIs evolve via
            // their own surface-stability metric (modularity).
            if c.kind == ContractKind::LibraryApi {
                continue;
            }
            contracts.push(CurrentContract {
                id: c.id.clone(),
                content_sha: c.fingerprint.clone(),
            });
        }

        // Every consumed-contract relationship contributes one
        // binding row. The binding's recorded
        // `derived_from_contract_sha` lives under the well-known
        // attribute key (Phase 1 analysers do not yet emit the key;
        // future phases populate it).
        for ic in &surfaces.contracts_consumed {
            let derived_from = ic
                .binding
                .attributes
                .get(DERIVED_FROM_CONTRACT_SHA_ATTR)
                .and_then(|v| v.as_str())
                .map(str::to_string);
            bindings.push(CurrentBinding {
                component: surfaces.component_id.clone(),
                contract_id: ic.contract_id.clone(),
                binding_content_sha: ic.binding.content_sha.clone(),
                derived_from_contract_sha: derived_from,
                language: ic.binding.language.clone(),
            });
        }
    }

    contracts.sort_by(|a, b| a.id.cmp(&b.id));
    bindings.sort_by(|a, b| {
        a.contract_id
            .cmp(&b.contract_id)
            .then_with(|| a.component.as_str().cmp(b.component.as_str()))
    });
    (contracts, bindings)
}

/// Resolve a component's on-disk `surfaces.yaml` path. Mirrors the
/// pipeline writer: walks `roots` for the first one whose
/// `<root>/<segment>` exists; falls back to `roots[0]` if none match.
fn resolve_surfaces_path(entry: &ComponentEntry, roots: &[PathBuf]) -> Option<PathBuf> {
    let seg = entry.path_segments.first()?;
    if seg.path.is_absolute() {
        return Some(seg.path.join(".atlas").join("cache").join("surfaces.yaml"));
    }
    for root in roots {
        let abs = root.join(&seg.path);
        if abs.exists() {
            return Some(abs.join(".atlas").join("cache").join("surfaces.yaml"));
        }
    }
    roots.first().map(|r| {
        r.join(&seg.path)
            .join(".atlas")
            .join("cache")
            .join("surfaces.yaml")
    })
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
