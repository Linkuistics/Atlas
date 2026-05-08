//! CLI plumbing for the four Phase 3 reports — `atlas drift`,
//! `atlas impact`, `atlas modularity`, `atlas divergence`.
//!
//! PR-7 shipped the clap argument structs and stub handlers; PR-8..PR-11
//! replace each handler body with the real load-database →
//! call-`atlas_reports::*` → render-or-persist flow.
//!
//! PR-8 (`atlas drift`) reads on-disk
//! `<root>/.atlas/cache/components.yaml` and each component's
//! `surfaces.yaml`, lifts contract+binding shas into
//! `atlas_reports::drift_pure`, then atomically writes the report and
//! the new snapshot. Reading the YAMLs (rather than recomputing the
//! engine database) matches design §3.1's "reports observe what the
//! engine has already produced" rule — no LLM calls, no fixedpoint.
//!
//! PR-9 (`atlas impact`) reads from
//! `Workspace::prior_components` / `Workspace::prior_related_components`
//! via a hard-error `ReportsBackend` to enforce the same no-engine-
//! recomputation invariant.
//!
//! PR-10 (`atlas modularity`) builds a full `AtlasDatabase` via
//! [`build_engine_database`] and runs the fixedpoint, then loads each
//! component's prior `modularity.yaml` history before calling
//! [`atlas_reports::modularity`]. Persistent LLM cache makes the
//! re-run near-free post-`atlas index`. Per-component files +
//! top-level rollup are written atomically via [`atomic_write`].
//!
//! PR-11 (`atlas divergence`) similarly builds a full database and
//! runs the fixedpoint. The drift snapshot is read read-only from
//! `<output>/.atlas/cache/contract-shas-snapshot.yaml`; the divergence
//! report is atomically written to
//! `<output>/.atlas/cache/reports/composition-divergence.yaml`. The
//! mechanism difference between PR-8/PR-9 (read from YAMLs) and
//! PR-10/PR-11 (run fixedpoint with cache) is acknowledged as
//! load-bearing for future converge-or-keep-divergent cleanup; PR-13's
//! polyglot smoke test is the ground-truth verifier (cold = Phase 2
//! baseline, warm = 0, report-runs = 0 LLM calls).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use atlas_engine::{
    all_components, atomic_write, expand_roots, run_fixedpoint, seed_filesystem_excluding,
    surface_of, AtlasDatabase, FixedpointConfig, LlmResponseCache, PersistentCache,
};
use atlas_index::{
    load_or_default_components, load_or_default_externals, load_or_default_overrides,
    load_or_default_related_components, load_or_default_subsystems_overrides, ComponentEntry,
    ComponentsFile, ContractKind, OverridesFile, SubsystemsOverridesFile, SurfacesFile,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};
use atlas_reports::{
    divergence, drift_pure, impact as run_impact_report, modularity, ComponentModularity,
    ContractShaSnapshot, CurrentBinding, CurrentContract, DivergenceReport, DriftReport,
    ImpactReport, ImpactReportTargetKind, ImpactTarget, ModularityHistory, ModularityReport,
    ReportError, ReportInputs, DERIVED_FROM_CONTRACT_SHA_ATTR,
};
use chrono::Utc;
use clap::ValueEnum;
use component_ontology::ComponentId;
use serde_json::Value;

use crate::backend::{self, compute_prompt_shas};
use crate::pipeline::{build_engine_database, resolve_component_dir, IndexConfig};
use crate::progress::{make_stderr_reporter, ProgressBackend, ProgressMode};
use crate::DEFAULT_OUTPUT_SUBDIR;

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
    /// Root of the codebase whose modularity to compute. Defaults to
    /// the current directory; mirrors `atlas index`'s positional
    /// argument so per-PR-10 the report subcommands share the same
    /// invocation shape.
    #[arg(default_value = ".")]
    pub root: PathBuf,

    /// Output format. Defaults to `yaml`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Yaml)]
    pub format: OutputFormat,

    /// Compute metrics but do not write per-component
    /// `modularity.yaml` files or the rollup.
    #[arg(long)]
    pub no_write: bool,

    /// LLM token budget for any cache misses below the surface /
    /// edges queries. Same `--budget` / `--no-budget` semantics as
    /// `atlas index`. Cache hits cost nothing; a fully-populated
    /// `.atlas/cache/` from a prior `atlas index` run is enough.
    #[arg(long)]
    pub budget: Option<u64>,

    /// Skip the budget check. Local development only.
    #[arg(long, conflicts_with = "budget")]
    pub no_budget: bool,
}

/// `atlas divergence` — pair-wise build-vs-deploy coupling check.
#[derive(Debug, clap::Args)]
pub struct DivergenceArgs {
    /// Root of the workspace to analyse. Defaults to `.`. Must point
    /// at a directory previously indexed by `atlas index` — the
    /// engine's persistent cache makes the divergence run near-free
    /// when so.
    #[arg(default_value = ".")]
    pub root: PathBuf,

    /// Where the four Atlas YAMLs live. Defaults to `<root>/.atlas/`.
    #[arg(long)]
    pub output_dir: Option<PathBuf>,

    /// Output format. Defaults to `yaml`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Yaml)]
    pub format: OutputFormat,

    /// Compute the divergence report but do not write
    /// `.atlas/cache/reports/composition-divergence.yaml`.
    #[arg(long)]
    pub no_write: bool,
}

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

/// `atlas impact <id>` — walk downstream consumers of a contract or
/// component and render a stdout-only impact report.
///
/// **Flow (Phase 3 plan §4 PR-9):**
///
/// 1. Resolve `<root>/.atlas/cache/components.yaml` and
///    `related-components.yaml` from disk. These are produced by
///    `atlas index`; if absent, both fall back to empty defaults and
///    the user gets a `target not found` error with empty suggestions.
/// 2. Construct a stand-in [`AtlasDatabase`] with both files installed
///    on the [`atlas_engine::Workspace`] inputs (`prior_components`,
///    `prior_related_components`). [`atlas_reports::impact`] reads
///    those inputs directly — no LLM pipeline runs.
/// 3. Resolve the user's `<id>` into an [`ImpactTarget`]: contract
///    namespace first, component namespace second.
/// 4. Call `atlas_reports::impact`. On `Ok`, render in the requested
///    format (default YAML; `--json` / `--human` toggle). Exit 0.
/// 5. On `Err(ReportError::TargetNotFound)`, print `target not found`
///    plus `did you mean:` candidates to stderr. Exit 2.
///
/// **`--no-write` is rejected at clap level** — the [`ImpactArgs`]
/// struct deliberately omits the flag, so `atlas impact --no-write
/// foo` produces a clap-emitted "unexpected argument" error and a
/// non-zero exit before this handler runs.
pub fn run_impact_cmd(args: ImpactArgs) -> Result<ExitCode> {
    // The CLI accepts a free-text id and, like `atlas index`, defaults
    // its working directory to the current working directory. The
    // user can override the output dir with $ATLAS_OUTPUT_DIR for
    // tests / non-default layouts; production callers point at
    // `<cwd>/.atlas`.
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    let output_dir = atlas_output_dir(&cwd);
    let cache_dir = output_dir.join("cache");
    let components_path = cache_dir.join("components.yaml");
    let related_path = cache_dir.join("related-components.yaml");

    let components = load_or_default_components(&components_path).with_context(|| {
        format!(
            "failed to load components.yaml at {}",
            components_path.display()
        )
    })?;
    let related = load_or_default_related_components(&related_path).with_context(|| {
        format!(
            "failed to load related-components.yaml at {}",
            related_path.display()
        )
    })?;

    let db = build_reports_database(&cwd, components, related);

    // Build inputs and resolve the target. Resolution order: contract
    // first, component second. We attempt a `ComponentId::parse` for
    // the component branch; an unparseable id falls through to the
    // contract branch, where the report function returns
    // `TargetNotFound` with Levenshtein-1 suggestions. This matches
    // the "two namespaces, disjoint by Phase 1 construction"
    // contract: a single `<id>` resolves into one or the other.
    let target = if id_is_known_contract(&db, &args.id) {
        ImpactTarget::Contract(args.id.clone())
    } else if let Ok(parsed) = ComponentId::parse(&args.id) {
        ImpactTarget::Component(parsed)
    } else {
        // Unparseable as a ComponentId and not a known contract; let
        // `impact()` fall through and emit Levenshtein candidates
        // against the union pool.
        ImpactTarget::Contract(args.id.clone())
    };

    let workspace = db.workspace();
    let inputs = ReportInputs {
        db: &db,
        workspace: &workspace,
    };

    match run_impact_report(inputs, target) {
        Ok(report) => {
            render_impact_report(&report, args.format)?;
            Ok(ExitCode::SUCCESS)
        }
        Err(ReportError::TargetNotFound { needle, candidates }) => {
            eprintln!("target not found: {needle}");
            if !candidates.is_empty() {
                eprintln!("did you mean:");
                for c in &candidates {
                    eprintln!("  - {c}");
                }
            }
            Ok(ExitCode::from(2))
        }
        Err(other) => Err(anyhow::anyhow!("impact report failed: {other}")),
    }
}

/// Resolve the Atlas output directory the way `atlas index` does:
/// `$ATLAS_OUTPUT_DIR` if set, else `<cwd>/.atlas`. Used by
/// [`run_impact_cmd`] so integration tests can point at a tempdir.
fn atlas_output_dir(cwd: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("ATLAS_OUTPUT_DIR") {
        PathBuf::from(p)
    } else {
        cwd.join(crate::DEFAULT_OUTPUT_SUBDIR)
    }
}

/// Build a minimal [`AtlasDatabase`] for report queries: a
/// [`ReportsBackend`] (never invoked because reports do not call the
/// LLM) plus the disk-loaded `components.yaml` /
/// `related-components.yaml` installed on the workspace's `prior_*`
/// slots, which is exactly what [`atlas_reports::impact`] reads from.
fn build_reports_database(
    root: &Path,
    components: atlas_index::ComponentsFile,
    related: atlas_index::RelatedComponentsFile,
) -> AtlasDatabase {
    let backend: Arc<dyn LlmBackend> = Arc::new(ReportsBackend);
    let mut db = AtlasDatabase::new(backend, vec![root.to_path_buf()], reports_fingerprint());
    db.set_prior_components(components);
    db.set_prior_related_components(related);
    db
}

/// Stand-in [`LlmFingerprint`] used by the report subcommands. The
/// fingerprint is never propagated into a real LLM call site (the
/// backend rejects every request) but [`AtlasDatabase::new`] requires
/// one on construction. Stable bytes keep two consecutive
/// `atlas impact` invocations on the same workspace deterministic.
fn reports_fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0u8; 32],
        ontology_sha: [0u8; 32],
        model_id: "atlas-reports/no-llm".into(),
        backend_version: "0".into(),
    }
}

/// LLM backend used for report subcommands. Reports do not call the
/// LLM — they read pre-computed engine state — but [`AtlasDatabase`]
/// requires a backend on construction. Any unexpected backend call
/// returns [`LlmError::Setup`] so a bug that triggers an LLM call
/// fails loudly instead of silently returning a stubbed response.
struct ReportsBackend;

impl LlmBackend for ReportsBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Setup(
            "reports backend should never be called; report subcommands read pre-computed state"
                .into(),
        ))
    }

    fn fingerprint(&self) -> LlmFingerprint {
        reports_fingerprint()
    }
}

/// `true` when `id` matches a contract id present in the prior
/// `related-components.yaml` (either as a `defines-contract` /
/// `implements-contract` / `consumes-contract` participant). We use
/// the same set [`atlas_reports::impact`] would consult when
/// resolving the target, so this check stays in sync with the report
/// function's resolution rule.
fn id_is_known_contract(db: &AtlasDatabase, id: &str) -> bool {
    let workspace = db.workspace();
    let related = workspace.prior_related_components(db as &dyn salsa::Database);
    for edge in &related.edges {
        // Contract participants live in `participants[1]` for the
        // contract-family kinds.
        let is_contract_kind = matches!(
            edge.kind,
            component_ontology::EdgeKind::DefinesContract
                | component_ontology::EdgeKind::ImplementsContract
                | component_ontology::EdgeKind::ConsumesContract
        );
        if is_contract_kind && edge.participants.get(1).map(|p| p.as_str()) == Some(id) {
            return true;
        }
    }
    false
}

/// Render an [`ImpactReport`] to stdout in the requested format. YAML
/// and JSON go through `serde_yaml` / `serde_json` directly; `Human`
/// emits an indented tree of consumers with per-axis annotations.
fn render_impact_report(report: &ImpactReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Yaml => {
            let yaml = serde_yaml::to_string(report).context("serialise impact report as YAML")?;
            print!("{yaml}");
        }
        OutputFormat::Json => {
            let json =
                serde_json::to_string_pretty(report).context("serialise impact report as JSON")?;
            println!("{json}");
        }
        OutputFormat::Human => {
            print_human(report);
        }
    }
    Ok(())
}

/// Indented-tree rendering of an impact report. The exact layout is
/// not part of the wire format (only YAML / JSON are), so the test for
/// `human` only asserts the presence of an indented consumer list.
fn print_human(report: &ImpactReport) {
    let kind = match report.target.kind {
        ImpactReportTargetKind::Contract => "contract",
        ImpactReportTargetKind::Component => "component",
    };
    println!("impact of {kind} `{}`:", report.target.id);
    println!("  direct consumers ({}):", report.summary.direct_count);
    if report.direct_consumers.is_empty() {
        println!("    (none)");
    } else {
        for c in &report.direct_consumers {
            println!("    - {c}");
        }
    }
    println!(
        "  transitive consumers ({}):",
        report.summary.transitive_count
    );
    if report.transitive_consumers.is_empty() {
        println!("    (none)");
    } else {
        for c in &report.transitive_consumers {
            println!("    - {c}");
        }
    }
    print_partition("by language", &report.partitions.by_language);
    print_partition("by deploy graph", &report.partitions.by_deploy_graph);
    print_partition("by lifecycle", &report.partitions.by_lifecycle);
}

fn print_partition(label: &str, map: &std::collections::BTreeMap<String, Vec<String>>) {
    println!("  {label}:");
    if map.is_empty() {
        println!("    (none)");
        return;
    }
    for (key, members) in map {
        println!("    {key}:");
        if members.is_empty() {
            println!("      (none)");
        } else {
            for m in members {
                println!("      - {m}");
            }
        }
    }
}

/// `atlas modularity` production entry point. Builds the engine
/// database against the live workspace via
/// [`build_engine_database`], then delegates to [`run_modularity`].
///
/// Mirrors `atlas index`'s budget posture (`--budget` mandatory unless
/// `--no-budget`) — a cold cache means LLM calls fire below the
/// surface / edges queries, and Atlas fails loud on runaway token
/// usage.
pub fn run_modularity_cmd(args: ModularityArgs) -> Result<ExitCode> {
    if args.budget.is_none() && !args.no_budget {
        anyhow::bail!(
            "`atlas modularity` requires `--budget <N-tokens>` to fail loudly on runaway LLM \
             usage (cold caches still fire LLM calls below the surface / edges queries). \
             Pass `--no-budget` for local development if you understand the risk."
        );
    }

    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;
    let output_dir = root.join(DEFAULT_OUTPUT_SUBDIR);
    let config_path = output_dir.join("config.yaml");
    let atlas_config = atlas_llm::AtlasConfig::load(&config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;

    let mut index_config = IndexConfig::new(root.clone());
    index_config.output_dir = output_dir.clone();
    index_config.prompt_shas = Some(compute_prompt_shas());

    let counter = args
        .budget
        .map(|b| Arc::new(atlas_llm::TokenCounter::new(b)));
    let reporter = make_stderr_reporter(ProgressMode::Auto, counter.clone());

    let observer = if reporter.drawing() {
        Some(Arc::clone(&reporter) as Arc<dyn atlas_llm::AgentObserver>)
    } else {
        None
    };

    let handles = crate::backend::build_production_backend_with_counter(
        &atlas_config,
        &index_config.root,
        counter.clone(),
        observer,
    )
    .context("failed to build LLM backend")?;
    index_config.fingerprint_override = Some(handles.fingerprint.clone());

    let backend: Arc<dyn atlas_llm::LlmBackend> =
        ProgressBackend::new(handles.backend.clone(), Arc::clone(&reporter))
            as Arc<dyn atlas_llm::LlmBackend>;

    let (db, roots) = build_engine_database(&index_config, backend, Arc::clone(&reporter))
        .context("failed to build engine database for modularity report")?;
    reporter.finish();

    let opts = ModularityRunOptions {
        format: args.format,
        no_write: args.no_write,
        roots,
        output_dir,
    };

    run_modularity(&db, &opts, &mut std::io::stdout().lock())?;

    drop(handles);
    Ok(ExitCode::SUCCESS)
}

/// `atlas divergence` entry-point. Resolves the production backend,
/// then forwards to [`run_divergence`]. The two-layer split lets
/// integration tests call [`run_divergence`] with a non-production
/// backend (e.g. [`atlas_llm::TestBackend`]) without requiring the
/// `claude` CLI to be on PATH.
pub fn run_divergence_cmd(args: DivergenceArgs) -> Result<ExitCode> {
    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| root.join(DEFAULT_OUTPUT_SUBDIR));

    let config_path = output_dir.join("config.yaml");
    let atlas_config = atlas_llm::AtlasConfig::load(&config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;
    let handles = backend::build_production_backend_with_counter(&atlas_config, &root, None, None)
        .context("failed to build LLM backend")?;

    let opts = DivergenceOptions {
        root,
        output_dir,
        no_write: args.no_write,
        format: args.format,
        fingerprint_override: None,
    };
    let report = run_divergence(&opts, handles.backend.clone())?;
    drop(handles);
    print_divergence_summary(&report);
    Ok(ExitCode::SUCCESS)
}

/// Knobs for [`run_divergence`]. Carved out from [`DivergenceArgs`] so
/// integration tests can construct the option set without going
/// through clap, and so the CLI handler can fold in computed paths
/// (canonicalised root, defaulted output dir) before the library call.
#[derive(Debug, Clone)]
pub struct DivergenceOptions {
    /// Canonicalised workspace root.
    pub root: PathBuf,
    /// Resolved `<root>/.atlas/` directory.
    pub output_dir: PathBuf,
    /// Skip the on-disk write of `composition-divergence.yaml`.
    pub no_write: bool,
    /// Output format for the rendered report (stdout).
    pub format: OutputFormat,
    /// Optional fingerprint override matching `atlas index`'s
    /// `IndexConfig::fingerprint_override`. When `None`, the backend's
    /// `fingerprint()` is installed verbatim.
    pub fingerprint_override: Option<LlmFingerprint>,
}

/// Build the engine, compute the divergence report, render it to
/// stdout, and (unless `no_write`) persist it to
/// `<output>/.atlas/cache/reports/composition-divergence.yaml`.
///
/// The engine setup mirrors `atlas index`'s prologue but skips the
/// LLM-budget gate, the writers for the four Atlas YAMLs, the
/// per-component cache projections, and the analyser-overrides walk
/// — the divergence report is read-only over the engine's outputs
/// and never advances any on-disk state besides
/// `composition-divergence.yaml` itself.
pub fn run_divergence(
    opts: &DivergenceOptions,
    backend: Arc<dyn LlmBackend>,
) -> Result<DivergenceReport> {
    let db = build_database_for_reports(
        &opts.root,
        &opts.output_dir,
        opts.fingerprint_override.clone(),
        backend,
    )?;

    // Read the drift snapshot read-only. The file may be absent (no
    // prior `atlas drift` run); a parse failure surfaces as a hard
    // error so users notice corruption rather than silently emitting
    // null-severity output.
    let snapshot_path = opts.output_dir.join("cache/contract-shas-snapshot.yaml");
    let drift_baseline = read_drift_snapshot_if_present(&snapshot_path)?;

    let workspace = db.workspace();
    let inputs = ReportInputs {
        db: &db,
        workspace: &workspace,
    };

    let report = divergence(inputs, drift_baseline.as_ref())
        .map_err(|e| anyhow::anyhow!("divergence report failed: {e}"))?;

    render_divergence(&report, opts.format)?;

    if !opts.no_write {
        let report_path = opts
            .output_dir
            .join("cache/reports/composition-divergence.yaml");
        let yaml = serde_yaml::to_string(&report)
            .context("failed to serialise composition-divergence.yaml")?;
        atomic_write(&report_path, yaml.as_bytes()).with_context(|| {
            format!(
                "failed to write composition-divergence.yaml to {}",
                report_path.display()
            )
        })?;
    }

    Ok(report)
}

/// Read `<output>/.atlas/cache/contract-shas-snapshot.yaml` (PR-8's
/// drift baseline). Absent file → `Ok(None)`. Present-but-corrupt →
/// `Err`. The function is intentionally read-only: divergence must
/// never advance the snapshot (regression guard:
/// `atlas_divergence_does_not_modify_drift_snapshot`).
fn read_drift_snapshot_if_present(path: &Path) -> Result<Option<ContractShaSnapshot>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read drift snapshot {}", path.display()))?;
    let parsed: ContractShaSnapshot = serde_yaml::from_slice(&bytes)
        .with_context(|| format!("failed to parse drift snapshot {}", path.display()))?;
    Ok(Some(parsed))
}

/// Construct an [`AtlasDatabase`] over `<root>`, seed the filesystem,
/// install the prior YAMLs (so the engine's rename-match and edge-
/// canonicalisation steps see the same baseline `atlas index` sees),
/// and run the fixedpoint. After the call the database's Salsa graph
/// is fully populated for reports to demand surfaces and edges
/// against.
fn build_database_for_reports(
    root: &Path,
    output_dir: &Path,
    fingerprint_override: Option<LlmFingerprint>,
    backend: Arc<dyn LlmBackend>,
) -> Result<AtlasDatabase> {
    // Mirror `run_index`'s prior-load + seed phases. The sub-paths
    // are the canonical Phase 3 cache locations (PR-4 / PR-5).
    let prior_components_path = output_dir.join("cache/components.yaml");
    let prior_externals_path = output_dir.join("external-components.yaml");
    let prior_related_path = output_dir.join("cache/related-components.yaml");
    let overrides_path = output_dir.join("components.overrides.yaml");
    let subsystems_overrides_path = output_dir.join("subsystems.overrides.yaml");

    let prior_components = load_or_default_components(&prior_components_path)?;
    let prior_externals = load_or_default_externals(&prior_externals_path)?;
    let prior_related = load_or_default_related_components(&prior_related_path)?;
    let overrides: OverridesFile = load_or_default_overrides(&overrides_path)?;
    let subsystems_overrides: SubsystemsOverridesFile =
        load_or_default_subsystems_overrides(&subsystems_overrides_path)?;

    let fingerprint = fingerprint_override.unwrap_or_else(|| backend.fingerprint());

    // Path-dep root expansion — same call `atlas index` makes so that
    // multi-root workspaces flow through identically.
    let auto_expanded = expand_roots(root).context("failed to expand path-dep roots")?;
    let roots: Vec<PathBuf> = if auto_expanded.is_empty() {
        vec![root.to_path_buf()]
    } else {
        auto_expanded
    };

    let mut db = AtlasDatabase::new(backend, roots.clone(), fingerprint);

    // Open the persistent on-disk cache. A failure here is non-fatal —
    // we degrade to in-memory cache. The CLI test pattern is "run
    // `atlas index` first, then `atlas divergence`" — the persistent
    // cache makes the divergence run a no-op LLM-call-wise.
    let persistent_cache_root = output_dir.join("cache");
    let llm_cache = match PersistentCache::open(&persistent_cache_root) {
        Ok(cache) => LlmResponseCache::new_with_persistent(cache),
        Err(_) => LlmResponseCache::new(),
    };
    db.set_llm_cache(llm_cache);

    // Match `run_index`'s exclusion shape: the primary root excludes
    // the output dir; peer roots get an empty exclusion.
    let mut excluded_dirs: Vec<PathBuf> = Vec::with_capacity(roots.len());
    excluded_dirs.push(output_dir.to_path_buf());
    for _ in 1..roots.len() {
        excluded_dirs.push(PathBuf::new());
    }
    seed_filesystem_excluding(&mut db, &roots, &excluded_dirs, true)
        .context("failed to seed filesystem")?;

    db.set_prior_components(prior_components);
    db.set_prior_externals(prior_externals);
    db.set_prior_related_components(prior_related);
    db.set_components_overrides(overrides);
    db.set_subsystems_overrides(subsystems_overrides);

    let fp_config = FixedpointConfig::default();
    let _fp_result = run_fixedpoint(&mut db, fp_config);

    // Pre-warm surfaces so the divergence report's `surface_artefacts_of`
    // calls land Salsa cache hits rather than restarting fixedpoint
    // dependencies. Without the pre-warm, the report's first contract-
    // sha lookup would block on a cold L5 surface query.
    let live: Vec<_> = all_components(&db)
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();
    for comp in &live {
        let _ = surface_of(&db, comp.id.clone());
    }

    Ok(db)
}

/// Render the report to stdout in the requested format. Errors are
/// surfaced verbatim — divergence's render path is the same shape PR-
/// 8/9/10 use for their own reports.
fn render_divergence(report: &DivergenceReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Yaml => {
            let yaml = serde_yaml::to_string(report)
                .context("failed to render divergence report as YAML")?;
            print!("{yaml}");
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(report)
                .context("failed to render divergence report as JSON")?;
            println!("{json}");
        }
        OutputFormat::Human => {
            print_divergence_human(report);
        }
    }
    Ok(())
}

/// Pretty-print the report for `--format human`. The format mirrors
/// the design §4.4 example layout.
fn print_divergence_human(report: &DivergenceReport) {
    println!("composition divergence report");
    println!("  generated_at: {}", report.generated_at);
    match report.drift_baseline_at {
        Some(t) => println!("  drift_baseline_at: {}", t),
        None => println!("  drift_baseline_at: <absent — severity is null for all pairs>"),
    }
    if report.divergent_pairs.is_empty() {
        println!("  no divergent pairs");
    } else {
        println!("  divergent pairs:");
        for pair in &report.divergent_pairs {
            let coupling = match pair.coupling {
                atlas_reports::DivergenceCoupling::BuildOnly => "build_only",
                atlas_reports::DivergenceCoupling::DeployOnly => "deploy_only",
            };
            let severity = match pair.severity {
                Some(s) => s.to_string(),
                None => "null".into(),
            };
            println!(
                "    {} <-> {}  ({}, severity {})",
                pair.components[0], pair.components[1], coupling, severity,
            );
            for c in &pair.drifting_contracts {
                println!("      drift: {c}");
            }
        }
    }
}

/// Print the run summary on stdout (the trailing single-line summary
/// the design spec §4.4 mandates: total pairs examined, divergent
/// count, by-severity histogram). Called after rendering so the
/// summary appears at the bottom of the output.
fn print_divergence_summary(report: &DivergenceReport) {
    let s = &report.summary;
    let mut hist_parts: Vec<String> = s
        .by_severity
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect();
    if hist_parts.is_empty() {
        hist_parts.push("(empty — no baseline)".into());
    }
    eprintln!(
        "atlas divergence: {} pairs examined; {} divergent; severity histogram: {{ {} }}",
        s.total_pairs_examined,
        s.divergent_count,
        hist_parts.join(", "),
    );
}

/// Library-shape options for [`run_modularity`]. The CLI binary
/// constructs one of these from clap-parsed flags + the resolved
/// roots; integration tests construct one directly.
#[derive(Debug, Clone)]
pub struct ModularityRunOptions {
    /// Output format selected by `--json | --yaml | --human`.
    pub format: OutputFormat,
    /// When `true`, [`run_modularity`] still computes the report and
    /// renders it but does not touch any cache file. Mirrors the CLI's
    /// `--no-write` flag.
    pub no_write: bool,
    /// Workspace roots, primary first. Used to resolve each
    /// component's on-disk directory for the per-component
    /// `<component>/.atlas/cache/modularity.yaml` write.
    pub roots: Vec<PathBuf>,
    /// Workspace output directory (`<primary-root>/.atlas`). The
    /// rollup lands at `<output_dir>/cache/reports/modularity-rollup.yaml`.
    pub output_dir: PathBuf,
}

/// Compute the modularity report off an already-built
/// [`AtlasDatabase`], rendering it to `out` and (unless
/// `opts.no_write`) writing per-component files + the top-level
/// rollup atomically.
///
/// Returns the in-memory [`ModularityReport`] for callers that want
/// to inspect the result; the same value is rendered to `out`.
pub fn run_modularity<W: Write>(
    db: &AtlasDatabase,
    opts: &ModularityRunOptions,
    out: &mut W,
) -> Result<ModularityReport> {
    // ---- prior history per component ---------------------------------
    // Plan §4 PR-10 step 2: walk every live component; for each, read
    // its prior `<component>/.atlas/cache/modularity.yaml` if present
    // and deserialise the `history` block into a `ModularityHistory`.
    let live_components = atlas_engine::all_components(db);
    let mut prior_per_component: HashMap<ComponentId, ModularityHistory> = HashMap::new();
    for entry in live_components.iter() {
        if entry.deleted {
            continue;
        }
        let Some(segment) = entry.path_segments.first() else {
            continue;
        };
        let component_dir = resolve_component_dir(&segment.path, &entry.manifests, &opts.roots);
        let prior_path = component_dir
            .join(".atlas")
            .join("cache")
            .join("modularity.yaml");
        if !prior_path.exists() {
            continue;
        }
        let bytes = match std::fs::read(&prior_path) {
            Ok(b) => b,
            Err(err) => {
                eprintln!(
                    "warning: failed to read {}: {err:#}; treating as no prior history",
                    prior_path.display()
                );
                continue;
            }
        };
        // The on-disk shape is the full `ComponentModularity` block;
        // we only need its `history` field for the prior-per-component
        // map. `ComponentModularity::history` deserialises directly,
        // and we accept the rest of the fields without re-validating
        // them (`schema_version` mismatch is fine — greenfield Phase 3
        // ships v1 only).
        match serde_yaml::from_slice::<ComponentModularity>(&bytes) {
            Ok(comp) => {
                prior_per_component.insert(
                    entry.id.clone(),
                    ModularityHistory {
                        entries: comp.history,
                    },
                );
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to parse {}: {err:#}; treating as no prior history",
                    prior_path.display()
                );
            }
        }
    }

    // ---- compute the report ------------------------------------------
    let workspace = db.workspace();
    let inputs = ReportInputs {
        db,
        workspace: &workspace,
    };
    let report = modularity(inputs, prior_per_component)
        .context("atlas_reports::modularity returned an error")?;

    // ---- render ------------------------------------------------------
    match opts.format {
        OutputFormat::Yaml => {
            let yaml = serde_yaml::to_string(&report).context("serialising modularity to YAML")?;
            out.write_all(yaml.as_bytes())
                .context("writing modularity YAML to output")?;
        }
        OutputFormat::Json => {
            let json =
                serde_json::to_string_pretty(&report).context("serialising modularity to JSON")?;
            out.write_all(json.as_bytes())
                .context("writing modularity JSON to output")?;
            out.write_all(b"\n").ok();
        }
        OutputFormat::Human => {
            render_modularity_human(&report, out).context("writing modularity human output")?;
        }
    }

    // ---- writes ------------------------------------------------------
    if !opts.no_write {
        // Per-component files. `atomic_write` mkdirs the parent chain.
        for entry in live_components.iter() {
            if entry.deleted {
                continue;
            }
            let Some(component) = report.per_component.get(&entry.id) else {
                continue;
            };
            let Some(segment) = entry.path_segments.first() else {
                continue;
            };
            let component_dir = resolve_component_dir(&segment.path, &entry.manifests, &opts.roots);
            let target = component_dir
                .join(".atlas")
                .join("cache")
                .join("modularity.yaml");
            let yaml = serde_yaml::to_string(component)
                .with_context(|| format!("failed to serialise modularity for {}", entry.id))?;
            atomic_write(&target, yaml.as_bytes())
                .with_context(|| format!("failed to write {}", target.display()))?;
        }

        // Top-level rollup. `<output_dir>/cache/reports/modularity-rollup.yaml`.
        let rollup_path = opts
            .output_dir
            .join("cache")
            .join("reports")
            .join("modularity-rollup.yaml");
        let rollup_yaml = serde_yaml::to_string(&report.rollup)
            .context("failed to serialise modularity rollup")?;
        atomic_write(&rollup_path, rollup_yaml.as_bytes())
            .with_context(|| format!("failed to write {}", rollup_path.display()))?;
    }

    // ---- summary ----------------------------------------------------
    let outlier_count: usize = report
        .rollup
        .subsystems
        .iter()
        .map(|s| s.outliers.len())
        .sum();
    eprintln!(
        "modularity: {} components, {} subsystems, {} outliers, {} unattached",
        report.per_component.len(),
        report.rollup.subsystems.len(),
        outlier_count,
        report.rollup.unattached_components.count,
    );

    Ok(report)
}

/// Render the modularity report in the `human` format — an indented
/// per-component breakdown followed by a subsystem rollup section.
/// Matches the design-spec §6.1 "indented text rendering" intent
/// without locking in a precise pixel layout (the JSON / YAML formats
/// are the canonical machine-readable shapes).
fn render_modularity_human<W: Write>(
    report: &ModularityReport,
    out: &mut W,
) -> std::io::Result<()> {
    writeln!(out, "modularity report")?;
    writeln!(out, "  generated_at: {}", report.generated_at)?;
    writeln!(out, "  components: {}", report.per_component.len())?;
    let mut ids: Vec<&ComponentId> = report.per_component.keys().collect();
    ids.sort();
    for id in ids {
        let comp = &report.per_component[id];
        let m = &comp.metrics;
        writeln!(out, "  - {}", id.as_str())?;
        writeln!(
            out,
            "      ca={} ce={} I={:.3} cohesion={:.3} stability={:.3} complexity={}",
            m.afferent_coupling,
            m.efferent_coupling,
            m.instability,
            m.cohesion,
            m.surface_stability,
            m.surface_complexity
        )?;
        writeln!(out, "      history_len={}", comp.history.len())?;
    }
    writeln!(out)?;
    writeln!(out, "subsystems: {}", report.rollup.subsystems.len())?;
    for sub in &report.rollup.subsystems {
        writeln!(
            out,
            "  - {} ({} members, {} outliers)",
            sub.id,
            sub.members.len(),
            sub.outliers.len()
        )?;
        let a = &sub.aggregates;
        writeln!(
            out,
            "      mean: ca={:.2} ce={:.2} I={:.3} cohesion={:.3} stability={:.3} complexity={:.2}",
            a.afferent_coupling.mean,
            a.efferent_coupling.mean,
            a.instability.mean,
            a.cohesion.mean,
            a.surface_stability.mean,
            a.surface_complexity.mean
        )?;
        for outlier in &sub.outliers {
            writeln!(
                out,
                "      OUTLIER {} on {}: value={:.3} (mean={:.3}, {:.2}σ)",
                outlier.component_id.as_str(),
                outlier.metric,
                outlier.value,
                outlier.subsystem_mean,
                outlier.deviation_sigmas,
            )?;
        }
    }
    let unatt = &report.rollup.unattached_components;
    writeln!(out, "unattached: {}", unatt.count)?;
    for id in &unatt.ids {
        writeln!(out, "  - {}", id.as_str())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Pure-shape tests for the renderers. The integration tests
    // exercising the full Salsa-driven flow live under
    // `crates/atlas-cli/tests/atlas_modularity.rs`.

    use atlas_reports::{
        ModularityHistoryEntry, ModularityHistoryMetrics, ModularityMetrics, ModularityRollup,
        SubsystemAggregate, SubsystemAggregateMetrics, SubsystemMetricStats, UnattachedComponents,
    };
    use chrono::TimeZone;

    fn cid(s: &str) -> ComponentId {
        ComponentId::parse(s).unwrap()
    }

    fn sample_report() -> ModularityReport {
        let mut per_component: HashMap<ComponentId, ComponentModularity> = HashMap::new();
        per_component.insert(
            cid("a/b"),
            ComponentModularity {
                schema_version: 1,
                component_id: cid("a/b"),
                generated_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 0, 0, 0).unwrap(),
                metrics: ModularityMetrics {
                    afferent_coupling: 1,
                    efferent_coupling: 2,
                    instability: 0.6667,
                    cohesion: 1.0,
                    surface_stability: 1.0,
                    surface_complexity: 4,
                },
                history: vec![ModularityHistoryEntry {
                    generated_at: chrono::Utc.with_ymd_and_hms(2026, 5, 7, 0, 0, 0).unwrap(),
                    surface_fingerprint: "fp-1".into(),
                    metrics: ModularityHistoryMetrics {
                        afferent_coupling: 1,
                        efferent_coupling: 2,
                        instability: 0.6667,
                        cohesion: 1.0,
                        surface_complexity: 4,
                    },
                }],
            },
        );
        ModularityReport {
            schema_version: 1,
            generated_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 0, 0, 0).unwrap(),
            per_component,
            rollup: ModularityRollup {
                schema_version: 1,
                generated_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 0, 0, 0).unwrap(),
                subsystems: vec![SubsystemAggregate {
                    id: "core".into(),
                    members: vec![cid("a/b")],
                    aggregates: SubsystemAggregateMetrics {
                        afferent_coupling: SubsystemMetricStats {
                            mean: 1.0,
                            stddev: 0.0,
                        },
                        efferent_coupling: SubsystemMetricStats {
                            mean: 2.0,
                            stddev: 0.0,
                        },
                        instability: SubsystemMetricStats {
                            mean: 0.6667,
                            stddev: 0.0,
                        },
                        cohesion: SubsystemMetricStats {
                            mean: 1.0,
                            stddev: 0.0,
                        },
                        surface_stability: SubsystemMetricStats {
                            mean: 1.0,
                            stddev: 0.0,
                        },
                        surface_complexity: SubsystemMetricStats {
                            mean: 4.0,
                            stddev: 0.0,
                        },
                    },
                    outliers: vec![],
                }],
                unattached_components: UnattachedComponents {
                    count: 0,
                    ids: vec![],
                },
            },
        }
    }

    #[test]
    fn human_format_renders_component_block_and_subsystem_rollup() {
        let report = sample_report();
        let mut buf: Vec<u8> = Vec::new();
        render_modularity_human(&report, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("a/b"),
            "human render must mention the component id; got: {s}"
        );
        assert!(
            s.contains("core"),
            "human render must mention the subsystem id; got: {s}"
        );
        assert!(
            s.contains("ca=1") && s.contains("ce=2"),
            "human render must surface raw metrics; got: {s}"
        );
    }
}
