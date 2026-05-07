//! End-to-end `atlas index` pipeline.
//!
//! ```text
//! 1. Load prior YAMLs from output-dir (components, externals,
//!    related-components, overrides).
//! 2. Build an AtlasDatabase seeded with:
//!    - the LLM backend (provided by the caller),
//!    - a fresh filesystem seed rooted at `config.root`,
//!    - the four prior YAMLs installed on Workspace inputs.
//! 3. Drive the fixedpoint (L8 back-edge loop).
//! 4. Demand the three L9 projections.
//! 5. On `--dry-run`, return the summary without writing anything.
//! 6. On budget exhaustion (detected via the driver's error or the
//!    counter's state), return `IndexError::BudgetExhausted` — the
//!    CLI maps that to exit code 2 and skips all writes.
//! 7. On an `LlmError::Setup` from any L3/L5/L6/L8 call, return
//!    `IndexError::SetupFailed` — exit code 3, no writes. The sentinel
//!    is consulted twice: once after the fixedpoint, once after the L9
//!    projection walk, so a Setup error first emitted during
//!    `surface_of` does not leak into the writer.
//! ```
//!
//! Atomic writes via `atlas_index::save_*_atomic`. The pipeline never
//! touches `components.overrides.yaml` — it is user-authored and lives
//! untouched.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use atlas_analyzers::AnalyzerRegistry;
use atlas_engine::{
    components_yaml_snapshot_with_prompt_shas, expand_roots, external_components_yaml_snapshot,
    per_component_yaml_snapshot, related_components_yaml_snapshot, run_fixedpoint,
    seed_filesystem_excluding, surfaces_yaml_snapshot, AtlasDatabase, FixedpointConfig,
    LlmResponseCache, PersistentCache, Phase, ProgressEvent, ProgressSink,
};
use atlas_index::{
    load_or_default_components, load_or_default_externals, load_or_default_overrides,
    load_or_default_related_components, load_or_default_subsystems,
    load_or_default_subsystems_overrides, save_components_atomic, save_externals_atomic,
    save_related_components_atomic, save_subsystems_atomic, AtlasConfigFile, ComponentsFile,
    OverridesFile, SubsystemsFile, SubsystemsOverridesFile,
};
use atlas_llm::{LlmBackend, LlmFingerprint, TokenCounter};

use crate::backend::BudgetSentinel;
use crate::timestamp::format_utc_rfc3339;

/// Default name for the directory that holds the four Atlas YAMLs.
/// Resolved relative to `config.root` unless `config.output_dir` is
/// set explicitly.
pub const DEFAULT_OUTPUT_SUBDIR: &str = ".atlas";

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("LLM token budget exhausted mid-run; no output files were written")]
    BudgetExhausted,

    /// The backend returned [`atlas_llm::LlmError::Setup`] for at least
    /// one call. Setup errors mean every call would fail the same way
    /// (e.g. config-load HTTP-provider rejection, missing CLI binary),
    /// so we abort the run instead of writing outputs derived from
    /// silent fallbacks. The string carries the first setup message the
    /// sentinel observed.
    #[error("LLM backend setup failed: {0}; no output files were written")]
    SetupFailed(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Runtime knobs for [`run_index`]. Constructed by the binary from
/// parsed command-line flags; tests fill one in by hand.
///
/// Atlas vNext is multi-root: `roots[0]` is the primary root (the
/// directory `atlas index` was invoked from); `additional_roots`
/// carries peer manifest-roots reached via path-dep walking (PR-4).
/// In Phase 1 the single-root case is still the natural common case
/// (the CLI defaults to `vec![primary]`); the multi-root code path is
/// dormant until path-dep expansion lands.
#[derive(Debug, Clone)]
pub struct IndexConfig {
    pub root: PathBuf,
    /// Peer roots beyond the primary `root`. Defaults to empty;
    /// PR-4's path-dep walk populates this. The full analysed set
    /// is `[root].iter().chain(additional_roots.iter())`.
    pub additional_roots: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub max_depth: u32,
    /// Bound on parallel `is_component` calls inside L8's map step.
    /// Plumbed through to [`atlas_engine::FixedpointConfig::map_concurrency`].
    pub map_concurrency: usize,
    pub recarve: bool,
    pub dry_run: bool,
    pub respect_gitignore: bool,
    /// Skip loading `components.overrides.yaml` and
    /// `subsystems.overrides.yaml` from the output dir. Files on disk
    /// are untouched. The fingerprint's `backend_version` is suffixed
    /// with `+overrides=disabled` so cache entries do not bleed
    /// between with/without runs.
    pub no_overrides: bool,
    /// Per-prompt SHA map embedded into `components.yaml`'s
    /// `cache_fingerprints.prompt_shas`. Left as `None` by tests that
    /// do not care; the CLI binary fills it from the embedded prompt
    /// corpus.
    pub prompt_shas: Option<std::collections::BTreeMap<String, String>>,
    /// Fingerprint to stamp onto the workspace input. When `None`,
    /// the backend's `fingerprint()` is installed verbatim.
    pub fingerprint_override: Option<LlmFingerprint>,
}

impl IndexConfig {
    /// Reasonable defaults for a command-line invocation: output
    /// directory is `<root>/.atlas/`, max depth per §8.2, no
    /// additional roots (single-root run).
    pub fn new(root: PathBuf) -> Self {
        let output_dir = root.join(DEFAULT_OUTPUT_SUBDIR);
        IndexConfig {
            root,
            additional_roots: Vec::new(),
            output_dir,
            max_depth: atlas_engine::DEFAULT_MAX_DEPTH,
            map_concurrency: atlas_engine::DEFAULT_MAP_CONCURRENCY,
            recarve: false,
            dry_run: false,
            respect_gitignore: true,
            no_overrides: false,
            prompt_shas: None,
            fingerprint_override: None,
        }
    }

    /// Full analysed root set, primary first. Equivalent to
    /// `[self.root.clone()] + self.additional_roots`; provided as a
    /// helper because every pipeline call site needs the same
    /// concatenation.
    pub fn all_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::with_capacity(1 + self.additional_roots.len());
        roots.push(self.root.clone());
        roots.extend(self.additional_roots.iter().cloned());
        roots
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSummary {
    pub component_count: usize,
    pub external_count: usize,
    pub edge_count: usize,
    pub llm_calls: u64,
    /// Number of cache misses where the backend returned an error.
    /// Distinct from `llm_calls` (successful misses) so a "0 calls,
    /// many errors" run is no longer misreported as a no-op success.
    pub llm_errors: u64,
    pub tokens_used: u64,
    pub token_budget: Option<u64>,
    pub fixedpoint_iterations: u32,
    pub outputs_written: bool,
}

/// Drive the engine end-to-end. `backend` is already wrapped with any
/// token counting the caller wants; `counter` is passed in so the
/// summary can report `tokens_used` without reaching into the backend.
/// The pipeline wraps `backend` in a [`BudgetSentinel`] internally, so
/// callers that build a backend by hand get exhaustion detection for
/// free — the sentinel observes every call.
pub fn run_index(
    config: &IndexConfig,
    backend: Arc<dyn LlmBackend>,
    counter: Option<Arc<TokenCounter>>,
    reporter: Arc<crate::progress::Reporter>,
) -> Result<IndexSummary, IndexError> {
    let sentinel = BudgetSentinel::new(backend);
    let backend: Arc<dyn LlmBackend> = sentinel.clone();

    let started_at = Instant::now();
    reporter.on_event(ProgressEvent::Started {
        root: config.root.clone(),
    });
    reporter.on_event(ProgressEvent::Phase(Phase::Seed));

    // ---- load prior outputs ---------------------------------------
    let prior_components_path = config.output_dir.join("components.yaml");
    let prior_externals_path = config.output_dir.join("external-components.yaml");
    let prior_related_path = config.output_dir.join("related-components.yaml");
    let overrides_path = config.output_dir.join("components.overrides.yaml");
    let subsystems_overrides_path = config.output_dir.join("subsystems.overrides.yaml");
    let subsystems_path = config.output_dir.join("subsystems.yaml");

    let prior_components =
        load_or_default_components(&prior_components_path).map_err(IndexError::Other)?;
    let prior_externals =
        load_or_default_externals(&prior_externals_path).map_err(IndexError::Other)?;
    let prior_related =
        load_or_default_related_components(&prior_related_path).map_err(IndexError::Other)?;
    let (overrides, subsystems_overrides) = if config.no_overrides {
        eprintln!(
            "atlas: --no-overrides is set; ignoring components.overrides.yaml and \
             subsystems.overrides.yaml (files on disk are untouched)"
        );
        (OverridesFile::default(), SubsystemsOverridesFile::default())
    } else {
        let overrides = load_or_default_overrides(&overrides_path).map_err(IndexError::Other)?;
        let subsystems_overrides = load_or_default_subsystems_overrides(&subsystems_overrides_path)
            .map_err(IndexError::Other)?;
        let validation =
            crate::validate::validate_overrides_with_subsystems(&overrides, &subsystems_overrides);
        if validation.has_any() {
            crate::validate::print_report(
                &validation,
                &overrides_path,
                &mut std::io::stderr().lock(),
            );
        }
        if validation.has_errors() {
            return Err(IndexError::Other(anyhow::anyhow!(
                "components.overrides.yaml has validation errors; fix them or run \
                 `atlas validate-overrides {}` for the full report",
                overrides_path.display()
            )));
        }
        (overrides, subsystems_overrides)
    };

    // ---- construct database ---------------------------------------
    let mut fingerprint = config
        .fingerprint_override
        .clone()
        .unwrap_or_else(|| backend.fingerprint());
    if config.no_overrides {
        fingerprint.backend_version.push_str("+overrides=disabled");
    }

    // PR-4: walk path-deps under the primary root to discover peer
    // manifest-roots automatically. The discovered roots are merged
    // with any manual `--additional-root` paths from the CLI; the
    // manual escape hatch coexists with the auto-discovery so users
    // can still extend the analysed set with paths that have no
    // path-dep edge (e.g. a sibling docs repo). Dedup is by
    // canonicalised path via `BTreeSet` so a manual flag pointing at
    // the same root the walk would have found does not double-count.
    let auto_expanded = expand_roots(&config.root).context("failed to expand path-dep roots")?;
    let auto_additional: Vec<PathBuf> = auto_expanded.iter().skip(1).cloned().collect();

    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let canonical_primary = auto_expanded
        .first()
        .cloned()
        .unwrap_or_else(|| config.root.clone());
    seen.insert(canonical_primary.clone());

    let mut all_additional: Vec<PathBuf> = Vec::new();
    // Manual `--additional-root` paths win the ordering tie: users
    // who explicitly listed a root expect it adjacent to the primary
    // in `components.yaml`. Auto-discovered peers follow.
    let manual_iter = config.additional_roots.iter().map(|r| (true, r));
    let auto_iter = auto_additional.iter().map(|r| (false, r));
    for (is_manual, r) in manual_iter.chain(auto_iter) {
        let canonical = match std::fs::canonicalize(r) {
            Ok(c) => c,
            Err(e) => {
                if is_manual {
                    // Manual --additional-root paths must canonicalise so
                    // dedup against the auto-discovered set works. A
                    // path that fails canonicalisation (missing,
                    // permission-denied, broken symlink) cannot be
                    // safely de-duplicated against the canonical set —
                    // skipping it is preferable to inserting a
                    // potentially-aliased non-canonical form.
                    eprintln!(
                        "warning: --additional-root {} could not be canonicalised: {}; skipping",
                        r.display(),
                        e
                    );
                }
                // Auto-discovered roots already came from `expand_roots`
                // which canonicalises internally, so a failure here is
                // a TOCTOU race; just skip silently.
                continue;
            }
        };
        if seen.insert(canonical.clone()) {
            all_additional.push(canonical);
        }
    }

    let mut roots: Vec<PathBuf> = Vec::with_capacity(1 + all_additional.len());
    roots.push(canonical_primary);
    roots.extend(all_additional.iter().cloned());

    // Persist the discovered roots to `<output>/.atlas/config.yaml`
    // for auditability (plan §4 PR-4). The file is otherwise
    // user-authored — preserve fields we don't own. A failure to
    // write here is non-fatal: the pipeline can still complete and
    // the user can always re-discover the roots by re-running.
    if !config.dry_run {
        if let Err(err) = persist_discovered_roots(&config.output_dir, &roots) {
            eprintln!(
                "warning: failed to persist discovered roots to {}: {err:#}",
                config.output_dir.join("config.yaml").display()
            );
        }
    }

    // PR-5: build the analyser registry. Start from the built-in
    // defaults (Cargo, Dockerfile, LLM-classify), then merge any
    // per-workspace overrides from `<output>/.atlas/analyzers.yaml`
    // — when present. A missing or unparseable file degrades to the
    // default registry with a warning so a typo does not break the
    // run.
    let mut registry = AnalyzerRegistry::builtin();
    let analyzers_yaml_path = config.output_dir.join("analyzers.yaml");
    if analyzers_yaml_path.exists() {
        match std::fs::read_to_string(&analyzers_yaml_path) {
            Ok(text) => match serde_yaml::from_str::<atlas_index::AnalyzersFile>(&text) {
                Ok(parsed) => registry.merge_yaml(&parsed),
                Err(e) => eprintln!(
                    "warning: failed to parse {}: {}; using built-in analyser defaults",
                    analyzers_yaml_path.display(),
                    e
                ),
            },
            Err(e) => eprintln!(
                "warning: failed to read {}: {}; using built-in analyser defaults",
                analyzers_yaml_path.display(),
                e
            ),
        }
    }
    let registry = std::sync::Arc::new(registry);

    let mut db = AtlasDatabase::new_with_registry(
        backend.clone(),
        roots.clone(),
        fingerprint.clone(),
        registry,
    );

    // PR-10: open the persistent content-addressed cache rooted at
    // `<output>/.atlas/cache/`. The on-disk layout is
    // `<output>/.atlas/cache/<stage>/<sha>.blob` (filesystem-native,
    // design §5.5 / §8.3). Open failures are non-fatal — we degrade
    // to an in-memory-only cache and warn, matching the policy used
    // by PR-4's config.yaml writer and PR-6's per-component writer.
    // Cache wiring is a perf feature; failing the run because the
    // cache dir is unwritable would be hostile.
    let persistent_cache_root = config.output_dir.join("cache");
    let llm_cache = match PersistentCache::open(&persistent_cache_root) {
        Ok(cache) => LlmResponseCache::new_with_persistent(cache),
        Err(err) => {
            eprintln!(
                "warning: failed to open persistent cache at {}: {err:#}; falling back to \
                 in-memory cache only (run completes; no cross-process cache hit)",
                persistent_cache_root.display()
            );
            LlmResponseCache::new()
        }
    };
    db.set_llm_cache(llm_cache);
    // The output_dir lives under the primary root only; peer roots
    // get an empty exclusion (no per-root output dir is written
    // beneath them in Phase 1). The slice positions matter — each
    // `excluded_dirs[i]` is paired with `roots[i]`.
    let mut excluded_dirs: Vec<PathBuf> = Vec::with_capacity(roots.len());
    excluded_dirs.push(config.output_dir.clone());
    for _ in 1..roots.len() {
        // Empty PathBuf is the no-op sentinel: excluded_relative_to silently
        // drops paths not under any root (canonicalize("") fails), so per-root
        // excluded vectors that lack an entry just contribute nothing.
        excluded_dirs.push(PathBuf::new());
    }
    seed_filesystem_excluding(&mut db, &roots, &excluded_dirs, config.respect_gitignore)
        .context("failed to seed filesystem")
        .map_err(IndexError::Other)?;

    if config.recarve {
        // Discard prior components so L4's rename-match does not anchor
        // the allocation cascade to stale ids. Externals and related
        // edges are re-derived downstream and are safe to carry.
        db.set_prior_components(ComponentsFile::default());
    } else {
        db.set_prior_components(prior_components.clone());
    }
    db.set_prior_externals(prior_externals);
    db.set_prior_related_components(prior_related);
    db.set_components_overrides(overrides);
    db.set_subsystems_overrides(subsystems_overrides.clone());

    // ---- fixedpoint -----------------------------------------------
    reporter.on_event(ProgressEvent::Phase(Phase::Fixedpoint));
    let sink: Arc<dyn atlas_engine::ProgressSink> = reporter.clone();
    let fp_config = FixedpointConfig {
        max_depth: config.max_depth,
        map_concurrency: config.map_concurrency,
        progress: Some(sink.clone()),
        ..FixedpointConfig::default()
    };
    let fp_result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_fixedpoint(&mut db, fp_config)
    })) {
        Ok(r) => r,
        Err(payload) => {
            if sentinel.was_exhausted() {
                return Err(IndexError::BudgetExhausted);
            }
            std::panic::resume_unwind(payload);
        }
    };

    if sentinel.was_exhausted() {
        return Err(IndexError::BudgetExhausted);
    }
    if sentinel.was_setup_failed() {
        return Err(IndexError::SetupFailed(
            sentinel
                .first_setup_message()
                .unwrap_or_else(|| "(no message)".to_string()),
        ));
    }

    // ---- demand L9 projections ------------------------------------
    reporter.on_event(ProgressEvent::Phase(Phase::Project));
    let live_components: Vec<_> = atlas_engine::all_components(&db)
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();

    // Post-L4 subsystem validation: cross-namespace collision and
    // id-form-member resolution. Both must be checked against the
    // resolved component tree, so they run after `all_components` is
    // available. Hard error on either; halt before any writes.
    if let Err(collisions) =
        atlas_engine::check_subsystem_namespace(&subsystems_overrides.subsystems, &live_components)
    {
        return Err(IndexError::Other(anyhow::anyhow!(
            "subsystem id(s) {:?} collide with component ids; rename the subsystem(s)",
            collisions
        )));
    }
    if let Err(bad) =
        atlas_engine::check_subsystem_id_members(&subsystems_overrides.subsystems, &live_components)
    {
        return Err(IndexError::Other(anyhow::anyhow!(
            "id-form member(s) {:?} do not resolve to any component (use a glob if the path is forward-looking)",
            bad
        )));
    }
    // surface_of is the run's slowest LLM-driven query on claude-code-only
    // stacks — each call is a heavy subprocess. Demanding them serially over
    // ~100 components turns the L9 pre-warm into hours. Mirror L8's map step
    // (`l8_recurse::run_map_step`): per-worker `db` clones via `map_with`,
    // because `&AtlasDatabase` is `!Send` (Salsa's `ZalsaLocal` is `!Sync`).
    let n = live_components.len() as u64;
    let map_concurrency = config.map_concurrency.max(1);
    if map_concurrency <= 1 || live_components.len() <= 1 {
        for (i, comp) in live_components.iter().enumerate() {
            reporter.on_event(ProgressEvent::Surface {
                component_id: comp.id.as_str().to_string(),
                relpath: atlas_engine::relpath_of(comp),
                k: (i as u64) + 1,
                n,
            });
            let _ = atlas_engine::surface_of(&db, comp.id.clone());
        }
    } else {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicU64, Ordering};
        let progress = AtomicU64::new(0);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(map_concurrency)
            .thread_name(|i| format!("atlas-l5-prewarm-{i}"))
            .build()
            .expect("rayon thread pool construction is infallible at sane sizes");
        let seed_db = db.clone();
        pool.install(|| {
            live_components
                .par_iter()
                .for_each_with(seed_db, |db_handle, comp| {
                    let k = progress.fetch_add(1, Ordering::Relaxed) + 1;
                    reporter.on_event(ProgressEvent::Surface {
                        component_id: comp.id.as_str().to_string(),
                        relpath: atlas_engine::relpath_of(comp),
                        k,
                        n,
                    });
                    let _ = atlas_engine::surface_of(db_handle, comp.id.clone());
                });
        });
    }
    let prompt_shas = config.prompt_shas.clone().unwrap_or_default();
    let mut components_file =
        (*components_yaml_snapshot_with_prompt_shas(&db, prompt_shas)).clone();
    let externals_file = (*external_components_yaml_snapshot(&db)).clone();
    reporter.on_event(ProgressEvent::Phase(Phase::Edges));
    let related_file = (*related_components_yaml_snapshot(&db)).clone();
    let mut subsystems_file = (*atlas_engine::subsystems_yaml_snapshot(&db)).clone();

    // Preserve generated_at for byte-identity on no-op re-runs: if
    // every other field of the new components file equals the prior
    // on-disk copy, reuse the prior timestamp.
    components_file.generated_at = stable_generated_at(
        prior_components_path.as_path(),
        &prior_components,
        &components_file,
        SystemTime::now(),
    );
    let prior_subsystems = load_or_default_subsystems(&subsystems_path).unwrap_or_default();
    subsystems_file.generated_at = stable_generated_at_subsystems(
        subsystems_path.as_path(),
        &prior_subsystems,
        &subsystems_file,
        SystemTime::now(),
    );

    // Final setup-error gate: the L5/L6/L9 walks above run after the
    // first sentinel check, so a Setup error first emitted during
    // surface_of or all_proposed_edges would otherwise reach the
    // writers. Re-check here so any setup failure aborts with no
    // outputs.
    if sentinel.was_setup_failed() {
        return Err(IndexError::SetupFailed(
            sentinel
                .first_setup_message()
                .unwrap_or_else(|| "(no message)".to_string()),
        ));
    }

    let summary = IndexSummary {
        component_count: components_file
            .components
            .iter()
            .filter(|c| !c.deleted)
            .count(),
        external_count: externals_file.externals.len(),
        edge_count: related_file.edges.len(),
        llm_calls: db.llm_cache().call_count(),
        llm_errors: db.llm_cache().error_count(),
        tokens_used: counter.as_ref().map(|c| c.used()).unwrap_or(0),
        token_budget: counter.as_ref().map(|c| c.budget()),
        fixedpoint_iterations: fp_result.iterations,
        outputs_written: !config.dry_run,
    };

    if !config.dry_run {
        std::fs::create_dir_all(&config.output_dir)
            .with_context(|| format!("failed to create {}", config.output_dir.display()))
            .map_err(IndexError::Other)?;

        save_components_atomic(&prior_components_path, &components_file)
            .map_err(IndexError::Other)?;
        save_externals_atomic(&prior_externals_path, &externals_file).map_err(IndexError::Other)?;
        save_related_components_atomic(&prior_related_path, &related_file)
            .map_err(IndexError::Other)?;
        save_subsystems_atomic(&subsystems_path, &subsystems_file).map_err(IndexError::Other)?;

        // PR-6: walk every component and write its per-component
        // `<component-path>/.atlas/component.yaml` projection. The
        // top-level `components.yaml` remains the canonical source;
        // per-component files are projections, so a failed write is
        // a degraded-but-correct state (warning on stderr, run
        // continues) rather than a hard error.
        write_per_component_files(&db, &components_file, &roots);

        // PR-10: the persistent cache is written-through inside
        // `LlmResponseCache::call_cached_with_fp` as each L-stage
        // call lands, so no end-of-pipeline flush is required. GC of
        // unreachable entries is deferred — Phase 1 ships an
        // unbounded cache; a future PR (or `atlas index --gc` flag)
        // can call `PersistentCache::gc(&mark_set)` against the
        // current run's contributing fingerprints.
    }

    // Finished fires AFTER the writes (or after the dry-run no-op) so a
    // consumer interpreting the event as "outputs are on disk" sees it
    // only once that is actually true. Spec §6.2 places the `done`
    // banner as the last line of scrollback for a successful run.
    reporter.on_event(ProgressEvent::Finished {
        components: summary.component_count as u64,
        llm_calls: summary.llm_calls,
        tokens_used: summary.tokens_used,
        token_budget: summary.token_budget,
        elapsed: started_at.elapsed(),
        breakdown: reporter.breakdown_snapshot(),
    });

    Ok(summary)
}

/// Decide what to stamp into `components.yaml::generated_at`. Returns
/// the prior value when the new snapshot equals what's already on
/// disk (modulo the timestamp itself); otherwise `now` formatted as
/// RFC3339.
fn stable_generated_at(
    prior_path: &Path,
    prior: &ComponentsFile,
    fresh: &ComponentsFile,
    now: SystemTime,
) -> String {
    if !prior_path.exists() {
        return format_utc_rfc3339(now);
    }
    let mut prior_canonical = prior.clone();
    let mut fresh_canonical = fresh.clone();
    prior_canonical.generated_at = String::new();
    fresh_canonical.generated_at = String::new();
    if prior_canonical == fresh_canonical && !prior.generated_at.is_empty() {
        prior.generated_at.clone()
    } else {
        format_utc_rfc3339(now)
    }
}

/// Mirror of [`stable_generated_at`] for `subsystems.yaml`. Reuses the
/// prior timestamp when the new snapshot equals what's on disk modulo
/// `generated_at`; otherwise stamps `now`.
fn stable_generated_at_subsystems(
    prior_path: &Path,
    prior: &SubsystemsFile,
    fresh: &SubsystemsFile,
    now: SystemTime,
) -> String {
    if !prior_path.exists() {
        return format_utc_rfc3339(now);
    }
    let mut prior_canonical = prior.clone();
    let mut fresh_canonical = fresh.clone();
    prior_canonical.generated_at = String::new();
    fresh_canonical.generated_at = String::new();
    if prior_canonical == fresh_canonical && !prior.generated_at.is_empty() {
        prior.generated_at.clone()
    } else {
        format_utc_rfc3339(now)
    }
}

/// Persist the discovered root set to `<output>/.atlas/config.yaml#roots`
/// (plan §4 PR-4 acceptance criterion). The file is otherwise
/// user-authored (operations / override_search are user knobs); we
/// load-or-default, overwrite only `roots`, and write back. The write
/// is via tempfile-then-rename so a crash mid-write cannot corrupt
/// the file.
fn persist_discovered_roots(output_dir: &Path, roots: &[PathBuf]) -> Result<()> {
    let path = output_dir.join("config.yaml");
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let mut existing = if path.exists() {
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        // The file may be hand-written and missing fields the schema
        // requires; degrade to default rather than refusing to update
        // `roots` because of an unrelated parse miss.
        serde_yaml::from_str::<AtlasConfigFile>(&contents).unwrap_or_default()
    } else {
        AtlasConfigFile::default()
    };
    existing.roots = roots.to_vec();
    let yaml = serde_yaml::to_string(&existing).context("failed to serialise config.yaml")?;
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy()
        .into_owned();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, yaml.as_bytes())
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Walk every (non-deleted) component in `components_file` and emit
/// per-component `<component-path>/.atlas/component.yaml` (PR-6) and
/// `<component-path>/.atlas/surfaces.yaml` (PR-7) files. The
/// component's on-disk path is resolved by joining its
/// first-`path_segments[0].path` against the matching root (longest
/// path-prefix among `roots`, via `best_root_for`). One `mkdir -p`
/// per component path; atomic write via tempfile-then-rename.
///
/// Per-component write failures are non-fatal warnings on stderr —
/// the top-level `components.yaml` is the canonical source, and
/// per-component files are projections. Continuing on failure
/// preserves the partial write (some components got their files,
/// some did not) rather than aborting the run after the top-level
/// writes have committed.
fn write_per_component_files(
    db: &AtlasDatabase,
    components_file: &ComponentsFile,
    roots: &[PathBuf],
) {
    for entry in &components_file.components {
        if entry.deleted {
            continue;
        }
        let segment = match entry.path_segments.first() {
            Some(seg) => seg,
            None => {
                eprintln!(
                    "warning: component `{}` has no path_segments; skipping per-component write",
                    entry.id.as_str()
                );
                continue;
            }
        };

        // Resolve the component's absolute on-disk directory. The
        // segment path is relative to one of `roots`; use the same
        // longest-prefix matcher as the engine.
        let candidate_abs = if segment.path.is_absolute() {
            segment.path.clone()
        } else {
            // Try every root in turn — the segment is relative to the
            // root that owns the component. We don't have explicit
            // ownership recorded on the segment, so probe by joining
            // and accepting the first root whose `<root>/<segment>`
            // exists. Fall back to the primary root if nothing
            // exists (the writer's mkdir will create whatever the
            // join names; the path will still be correct because
            // segment paths are unambiguous under their root).
            let mut chosen: Option<PathBuf> = None;
            for root in roots {
                let candidate = root.join(&segment.path);
                if candidate.exists() {
                    chosen = Some(candidate);
                    break;
                }
            }
            match chosen {
                Some(p) => p,
                None => roots
                    .first()
                    .map(|r| r.join(&segment.path))
                    .unwrap_or_else(|| segment.path.clone()),
            }
        };

        let target_dir = candidate_abs.join(".atlas");

        // -- component.yaml (PR-6) -----------------------------------
        // Note: per_component_yaml_snapshot now consults
        // surfaces_yaml_snapshot for its fingerprint (PR-7), so
        // calling it transitively produces the surface artefacts
        // already. That call is cheap to repeat below thanks to
        // Salsa's memoisation of the underlying surface_of inputs.
        let component_snapshot = match per_component_yaml_snapshot(db, &entry.id) {
            Ok(arc) => arc,
            Err(err) => {
                eprintln!(
                    "warning: failed to project component `{}`: {err:#}; skipping per-component write",
                    entry.id.as_str()
                );
                continue;
            }
        };

        let component_file_path = target_dir.join("component.yaml");
        if let Err(err) =
            write_per_component_atomic(&target_dir, &component_file_path, &component_snapshot)
        {
            eprintln!(
                "warning: failed to write {}: {err:#}; the top-level components.yaml is unaffected",
                component_file_path.display()
            );
        }

        // -- surfaces.yaml (PR-7) ------------------------------------
        // Surfaces are projections too: a failed write is a non-fatal
        // warning. The top-level components.yaml does not (yet) carry
        // surface fingerprints, so a missing surfaces.yaml degrades
        // L6 cache invalidation across components but does not break
        // the canonical output.
        let surfaces_snapshot = match surfaces_yaml_snapshot(db, &entry.id) {
            Ok(arc) => arc,
            Err(err) => {
                eprintln!(
                    "warning: failed to project surfaces for `{}`: {err:#}; skipping per-component surfaces.yaml write",
                    entry.id.as_str()
                );
                continue;
            }
        };

        let surfaces_file_path = target_dir.join("surfaces.yaml");
        if let Err(err) = write_yaml_atomic(&target_dir, &surfaces_file_path, &*surfaces_snapshot) {
            // Note: `&*surfaces_snapshot` (deref of the Arc) is
            // required here because `write_yaml_atomic` is generic on
            // `T: Serialize`; auto-deref does not select the inner
            // `SurfacesFile` impl through `Arc<T>` for generic
            // monomorphisation. The component-side equivalent above
            // works without it because that helper is non-generic.
            eprintln!(
                "warning: failed to write {}: {err:#}; the top-level components.yaml is unaffected",
                surfaces_file_path.display()
            );
        }
    }
}

fn write_per_component_atomic(
    target_dir: &Path,
    target_file: &Path,
    file: &atlas_index::PerComponentFile,
) -> Result<()> {
    write_yaml_atomic(target_dir, target_file, file)
}

/// Generic atomic-write helper for any serde-serialisable value.
/// Used by both the component.yaml (PR-6) and surfaces.yaml (PR-7)
/// writers so the temp-file naming, mkdir-p, rename pattern is
/// consistent.
fn write_yaml_atomic<T: serde::Serialize>(
    target_dir: &Path,
    target_file: &Path,
    file: &T,
) -> Result<()> {
    std::fs::create_dir_all(target_dir)
        .with_context(|| format!("failed to create {}", target_dir.display()))?;
    let yaml = serde_yaml::to_string(file)
        .with_context(|| format!("failed to serialise {}", target_file.display()))?;
    let file_name = target_file
        .file_name()
        .with_context(|| format!("{} has no file name", target_file.display()))?
        .to_string_lossy()
        .into_owned();
    let tmp = target_dir.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, yaml.as_bytes())
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, target_file).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp.display(),
            target_file.display()
        )
    })?;
    Ok(())
}

/// Helper the binary uses to write the one-line summary banner.
pub fn format_summary(summary: &IndexSummary) -> String {
    let budget = summary
        .token_budget
        .map(|b| format!("{}/{}", summary.tokens_used, b))
        .unwrap_or_else(|| format!("{} (no budget)", summary.tokens_used));
    format!(
        "atlas index: components={} externals={} edges={} llm_calls={} llm_errors={} tokens={} iterations={} written={}",
        summary.component_count,
        summary.external_count,
        summary.edge_count,
        summary.llm_calls,
        summary.llm_errors,
        budget,
        summary.fixedpoint_iterations,
        summary.outputs_written,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn components_file_with(generated_at: &str) -> ComponentsFile {
        ComponentsFile {
            generated_at: generated_at.to_string(),
            ..ComponentsFile::default()
        }
    }

    #[test]
    fn stable_generated_at_returns_prior_when_content_unchanged() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("components.yaml");
        std::fs::write(&path, "ignored").unwrap();

        let prior = components_file_with("2025-04-24T00:00:00Z");
        let fresh = components_file_with("ignored-because-overridden");
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_745_452_800);

        let result = stable_generated_at(&path, &prior, &fresh, now);

        assert_eq!(result, "2025-04-24T00:00:00Z");
    }

    #[test]
    fn stable_generated_at_uses_now_when_content_differs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("components.yaml");
        std::fs::write(&path, "ignored").unwrap();

        let prior = components_file_with("2024-01-01T00:00:00Z");
        let mut fresh = components_file_with("overridden");
        fresh.cache_fingerprints.model_id = "different-model".into();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_745_452_800);

        let result = stable_generated_at(&path, &prior, &fresh, now);

        assert_eq!(result, "2025-04-24T00:00:00Z");
    }

    #[test]
    fn stable_generated_at_uses_now_on_first_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("components.yaml"); // does not exist
        let prior = components_file_with("");
        let fresh = components_file_with("overridden");
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_745_452_800);

        let result = stable_generated_at(&path, &prior, &fresh, now);

        assert_eq!(result, "2025-04-24T00:00:00Z");
    }
}
