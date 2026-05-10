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
    all_components, atomic_write, components_yaml_snapshot_with_prompt_shas,
    ensure_atlas_gitignore, external_components_yaml_snapshot, per_component_yaml_snapshot,
    related_components_yaml_snapshot, run_fixedpoint, seed_filesystem_excluding,
    surfaces_yaml_snapshot, AtlasDatabase, FixedpointConfig, LlmResponseCache, PersistentCache,
    Phase, ProgressEvent, ProgressSink,
};
use atlas_index::{
    load_or_default_components, load_or_default_externals, load_or_default_overrides,
    load_or_default_related_components, load_or_default_subsystems,
    load_or_default_subsystems_overrides, save_components_atomic, save_externals_atomic,
    save_subsystems_atomic, AtlasConfigFile, ComponentsFile, OverridesFile, SubsystemsFile,
    SubsystemsOverridesFile,
};
use atlas_llm::{LlmBackend, LlmFingerprint, TokenCounter};
use component_ontology::validate_contract_participants_resolve;

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

/// Per-session dedup for [`ensure_atlas_gitignore`] calls.
///
/// Phase 3 PR-1 (plan §4 PR-1, design §5.6): every `.atlas/` scope
/// gets a one-line `.gitignore` listing `cache/` so the per-scope
/// cache directories introduced by PR-2..PR-5 are not accidentally
/// committed. The check is idempotent on disk, but we still want to
/// avoid re-walking and re-warning about the same scope on every
/// individual `.atlas/` write — `dedup` records canonicalised scopes
/// so the warning emits at most once per session per scope.
#[derive(Default)]
struct GitignoreSession {
    seen: BTreeSet<PathBuf>,
}

impl GitignoreSession {
    fn ensure(&mut self, scope: &Path) {
        // Use the canonicalised scope key so multiple write points that
        // pass equivalent (but textually different) paths still dedup.
        // Fall back to the input path when canonicalisation fails — a
        // brand-new scope might not exist on disk yet at the moment of
        // the first call, in which case `canonicalize` errors and we
        // dedup on the literal path. Either is fine for the warning's
        // "at most once per scope" intent.
        let key = std::fs::canonicalize(scope).unwrap_or_else(|_| scope.to_path_buf());
        if !self.seen.insert(key) {
            return;
        }
        if let Err(err) = ensure_atlas_gitignore(scope) {
            eprintln!(
                "warning: failed to write .atlas/.gitignore at {}: {err}; \
                 cache files may be tracked unintentionally",
                scope.display()
            );
        }
    }
}

/// Runtime knobs for [`run_index`]. Constructed by the binary from
/// parsed command-line flags; tests fill one in by hand.
#[derive(Debug, Clone)]
pub struct IndexConfig {
    pub root: PathBuf,
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
    /// directory is `<root>/.atlas/`, max depth per §8.2.
    pub fn new(root: PathBuf) -> Self {
        let output_dir = root.join(DEFAULT_OUTPUT_SUBDIR);
        IndexConfig {
            root,
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
    // PR-4 (Phase 3): top-level `components.yaml` moved to
    // `<output>/.atlas/cache/components.yaml`. The reader here and the
    // writer below both point at the cache sub-path so prior-run
    // rename-match continues to work.
    let prior_components_path = config.output_dir.join("cache/components.yaml");
    let prior_externals_path = config.output_dir.join("external-components.yaml");
    // PR-5 (Phase 3): related-components.yaml is a derived/cache file;
    // its canonical location is now <output>/.atlas/cache/related-components.yaml.
    let prior_related_path = config.output_dir.join("cache/related-components.yaml");
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

    let roots: Vec<PathBuf> = vec![config.root.clone()];

    // PR-1 (Phase 3): the workspace `.atlas/` scope is `output_dir`'s
    // parent; the gitignore goes into `<scope>/.atlas/.gitignore`. We
    // call this before the very first `.atlas/` write so retrofit
    // cache files (PR-2..PR-5) are never tracked.
    let mut gitignore_session = GitignoreSession::default();
    let workspace_scope = config
        .output_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Persist the workspace root to `<output>/.atlas/config.yaml`
    // for auditability (plan §4 PR-4). The file is otherwise
    // user-authored — preserve fields we don't own. A failure to
    // write here is non-fatal: the pipeline can still complete.
    if !config.dry_run {
        gitignore_session.ensure(&workspace_scope);
        if let Err(err) = persist_workspace_root(&config.output_dir, &config.root) {
            eprintln!(
                "warning: failed to persist workspace root to {}: {err:#}",
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
        config.root.clone(),
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
    // The output_dir lives under the workspace root; excluded_dirs[0]
    // is paired with roots[0] by seed_filesystem_excluding's contract.
    let excluded_dirs: Vec<PathBuf> = vec![config.output_dir.clone()];
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

    // PR-8: validate that every contract participant in the emitted
    // `related-components.yaml` resolves to a contract defined in some
    // live component's `surfaces.yaml`. Runs unconditionally — the
    // same as the subsystem validators above — because a dry-run must
    // surface every failure the real run would. The validator is purely
    // structural: no I/O, no writes. On failure the function returns
    // early; `outputs_written` is `false` because no files are touched
    // (the writes block below is never reached on this error path,
    // whether the caller requested a dry-run or not).
    {
        let live_ids: Vec<component_ontology::ComponentId> = all_components(&db)
            .iter()
            .filter(|c| !c.deleted)
            .filter_map(|c| component_ontology::ComponentId::parse(c.id.as_str()).ok())
            .collect();
        // Collect every contract id defined in any live component's
        // surfaces.yaml. `surfaces_yaml_snapshot` is Salsa-memoised —
        // these calls are cheap after the L9 walk above pre-warmed them.
        let known_ids_owned: Vec<String> = live_ids
            .iter()
            .filter_map(|cid| surfaces_yaml_snapshot(&db, cid).ok())
            .flat_map(|sf| {
                sf.contracts_defined
                    .iter()
                    .map(|c| c.id.clone())
                    .collect::<Vec<_>>()
            })
            .collect();
        let known_contract_ids: std::collections::BTreeSet<&str> =
            known_ids_owned.iter().map(String::as_str).collect();
        if let Err(unresolved) =
            validate_contract_participants_resolve(&related_file.edges, &known_contract_ids)
        {
            let lines: Vec<String> = unresolved
                .iter()
                .map(|u| {
                    format!(
                        "  {} edge: component `{}` → unresolved contract `{}`",
                        u.edge_kind.as_str(),
                        u.component_participant,
                        u.unresolved_contract_id,
                    )
                })
                .collect();
            return Err(IndexError::Other(anyhow::anyhow!(
                "related-components.yaml contains {} unresolved contract participant(s):\n{}",
                unresolved.len(),
                lines.join("\n"),
            )));
        }
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

        // PR-4 (Phase 3): ensure the cache sub-directory exists before
        // writing `cache/components.yaml`. The persistent LLM cache
        // (PR-10) also lives here; `PersistentCache::open` creates this
        // directory on success, but we may have fallen back to in-memory
        // if open failed. An explicit mkdir-p makes the write
        // unconditional.
        std::fs::create_dir_all(config.output_dir.join("cache"))
            .with_context(|| {
                format!(
                    "failed to create cache directory {}",
                    config.output_dir.join("cache").display()
                )
            })
            .map_err(IndexError::Other)?;

        // PR-1 (Phase 3): make sure the workspace-scope `.gitignore`
        // is in place before we write any of the canonical YAMLs. The
        // session dedups against the earlier call sites (config.yaml,
        // any future write points) so this is a no-op walk on the
        // hot path.
        gitignore_session.ensure(&workspace_scope);

        save_components_atomic(&prior_components_path, &components_file)
            .map_err(IndexError::Other)?;
        save_externals_atomic(&prior_externals_path, &externals_file).map_err(IndexError::Other)?;
        // PR-5 (Phase 3): write to cache/ subdir via atomic_write (design §6.3).
        // `atomic_write` creates the parent directory chain so the cache/
        // subdirectory is guaranteed to exist before the rename lands.
        {
            let related_yaml = serde_yaml::to_string(&related_file)
                .context("failed to serialise related-components to YAML")
                .map_err(IndexError::Other)?;
            atomic_write(&prior_related_path, related_yaml.as_bytes())
                .context("failed to write cache/related-components.yaml")
                .map_err(IndexError::Other)?;
        }

        save_subsystems_atomic(&subsystems_path, &subsystems_file).map_err(IndexError::Other)?;

        // PR-6 / PR-3: walk every component and write its per-component
        // `<component-path>/.atlas/cache/component.yaml` projection.
        // The top-level `components.yaml` remains the canonical source;
        // per-component files are projections, so a failed write is
        // a degraded-but-correct state (warning on stderr, run
        // continues) rather than a hard error.
        write_per_component_files(&db, &components_file, &roots, &mut gitignore_session);

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

/// Build an [`AtlasDatabase`] up to and including the L4–L6 fixedpoint
/// plus an L5 surface pre-warm, returning the populated database
/// alongside the workspace root the database was seeded with
/// (canonicalised by the caller before being passed in). Skips all of
/// [`run_index`]'s post-fixedpoint writes, the
/// validators that inspect related-components / subsystem id-form
/// members, and the `IndexSummary` accounting.
///
/// Used by the read-only Phase 3 report subcommands (`atlas drift`,
/// `atlas modularity`, `atlas divergence`) which need a fully-driven
/// `AtlasDatabase` but do not produce the four canonical YAMLs. The
/// flow here is intentionally a focused subset of [`run_index`]:
///
/// 1. Load prior YAMLs (components / externals / related / overrides /
///    subsystems-overrides).
/// 2. Resolve the single workspace root from `config.root`.
/// 3. Build the analyser registry from the in-tree default + any
///    `<output>/.atlas/analyzers.yaml` overrides.
/// 4. Construct the [`AtlasDatabase`], install the persistent LLM
///    cache (in-memory fallback on open failure), seed the filesystem.
/// 5. Run the L4–L6 fixedpoint via [`run_fixedpoint`].
/// 6. Pre-warm L5 surface_of for every live component (mirrors the
///    parallel walk in [`run_index`]).
///
/// Reports compute their own metric blocks downstream; they do not need
/// the L9 projections (`components.yaml`, etc.). The persistent LLM
/// cache is opened so that a second `atlas modularity` invocation
/// behind an already-populated cache does not re-spend tokens.
pub fn build_engine_database(
    config: &IndexConfig,
    backend: Arc<dyn LlmBackend>,
    reporter: Arc<crate::progress::Reporter>,
) -> Result<(AtlasDatabase, Vec<PathBuf>), IndexError> {
    let sentinel = BudgetSentinel::new(backend);
    let backend: Arc<dyn LlmBackend> = sentinel.clone();

    reporter.on_event(ProgressEvent::Started {
        root: config.root.clone(),
    });
    reporter.on_event(ProgressEvent::Phase(Phase::Seed));

    // ---- load prior outputs ---------------------------------------
    let prior_components_path = config.output_dir.join("cache/components.yaml");
    let prior_externals_path = config.output_dir.join("external-components.yaml");
    let prior_related_path = config.output_dir.join("cache/related-components.yaml");
    let overrides_path = config.output_dir.join("components.overrides.yaml");
    let subsystems_overrides_path = config.output_dir.join("subsystems.overrides.yaml");

    let prior_components =
        load_or_default_components(&prior_components_path).map_err(IndexError::Other)?;
    let prior_externals =
        load_or_default_externals(&prior_externals_path).map_err(IndexError::Other)?;
    let prior_related =
        load_or_default_related_components(&prior_related_path).map_err(IndexError::Other)?;
    let (overrides, subsystems_overrides) = if config.no_overrides {
        (OverridesFile::default(), SubsystemsOverridesFile::default())
    } else {
        let overrides = load_or_default_overrides(&overrides_path).map_err(IndexError::Other)?;
        let subsystems_overrides = load_or_default_subsystems_overrides(&subsystems_overrides_path)
            .map_err(IndexError::Other)?;
        (overrides, subsystems_overrides)
    };

    let mut fingerprint = config
        .fingerprint_override
        .clone()
        .unwrap_or_else(|| backend.fingerprint());
    if config.no_overrides {
        fingerprint.backend_version.push_str("+overrides=disabled");
    }

    let roots: Vec<PathBuf> = vec![config.root.clone()];

    // ---- analyser registry ----------------------------------------
    let mut registry = AnalyzerRegistry::builtin();
    let analyzers_yaml_path = config.output_dir.join("analyzers.yaml");
    if analyzers_yaml_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&analyzers_yaml_path) {
            if let Ok(parsed) = serde_yaml::from_str::<atlas_index::AnalyzersFile>(&text) {
                registry.merge_yaml(&parsed);
            }
        }
    }
    let registry = std::sync::Arc::new(registry);

    // ---- construct + seed database --------------------------------
    let mut db = AtlasDatabase::new_with_registry(
        backend.clone(),
        config.root.clone(),
        fingerprint.clone(),
        registry,
    );

    let persistent_cache_root = config.output_dir.join("cache");
    let llm_cache = match PersistentCache::open(&persistent_cache_root) {
        Ok(cache) => LlmResponseCache::new_with_persistent(cache),
        Err(_) => LlmResponseCache::new(),
    };
    db.set_llm_cache(llm_cache);

    let excluded_dirs: Vec<PathBuf> = vec![config.output_dir.clone()];
    seed_filesystem_excluding(&mut db, &roots, &excluded_dirs, config.respect_gitignore)
        .context("failed to seed filesystem")
        .map_err(IndexError::Other)?;

    if config.recarve {
        db.set_prior_components(ComponentsFile::default());
    } else {
        db.set_prior_components(prior_components.clone());
    }
    db.set_prior_externals(prior_externals);
    db.set_prior_related_components(prior_related);
    db.set_components_overrides(overrides);
    db.set_subsystems_overrides(subsystems_overrides);

    // ---- fixedpoint -----------------------------------------------
    reporter.on_event(ProgressEvent::Phase(Phase::Fixedpoint));
    let sink: Arc<dyn atlas_engine::ProgressSink> = reporter.clone();
    let fp_config = FixedpointConfig {
        max_depth: config.max_depth,
        map_concurrency: config.map_concurrency,
        progress: Some(sink.clone()),
        ..FixedpointConfig::default()
    };
    let fp_outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_fixedpoint(&mut db, fp_config)
    }));
    match fp_outcome {
        Ok(_r) => {}
        Err(payload) => {
            if sentinel.was_exhausted() {
                return Err(IndexError::BudgetExhausted);
            }
            std::panic::resume_unwind(payload);
        }
    }

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

    // ---- L5 surface pre-warm --------------------------------------
    reporter.on_event(ProgressEvent::Phase(Phase::Project));
    let live_components: Vec<_> = atlas_engine::all_components(&db)
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();
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

    if sentinel.was_setup_failed() {
        return Err(IndexError::SetupFailed(
            sentinel
                .first_setup_message()
                .unwrap_or_else(|| "(no message)".to_string()),
        ));
    }

    Ok((db, roots))
}

/// Thin wrapper around [`build_engine_database`] for the read-only
/// `atlas divergence` handler. PR-11 of Phase 3 introduced a private
/// `build_database_for_reports` helper inside `reports.rs` that
/// duplicated ~75 lines of [`build_engine_database`]'s body via a
/// slightly leaner code path (no [`BudgetSentinel`], no
/// [`crate::progress::Reporter`] events, no analyser-overrides merge,
/// sequential L5 pre-warm, default `IndexConfig` flags). Phase 4 PR-5
/// converged the two: this wrapper synthesises an [`IndexConfig`] with
/// the same defaults the helper used (no `--no-overrides`, no
/// `--recarve`, `respect_gitignore = true`) and forwards through to
/// [`build_engine_database`], which is a strict semantic superset.
///
/// The wrapper exists rather than baking divergence-specific knobs into
/// [`build_engine_database`] because the canonical helper is correct
/// for both code paths: gaining BudgetSentinel coverage, the
/// analyser-overrides merge, and the parallel L5 pre-warm in the
/// divergence path is upside, not drift.
///
/// Returns the populated database, discarding the resolved roots
/// (`run_divergence` re-derives them via [`AtlasDatabase::workspace`]).
pub fn build_engine_database_for_reports(
    root: &Path,
    output_dir: &Path,
    fingerprint_override: Option<LlmFingerprint>,
    backend: Arc<dyn LlmBackend>,
) -> Result<AtlasDatabase> {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = output_dir.to_path_buf();
    config.fingerprint_override = fingerprint_override;

    // The reports path predates the progress reporter; emit a silent
    // reporter so the call site stays event-free (the divergence
    // handler renders its own summary on stderr after the fact).
    let reporter =
        crate::progress::make_stderr_reporter(crate::progress::ProgressMode::Never, None);

    let (db, _roots) = build_engine_database(&config, backend, reporter).map_err(|e| match e {
        IndexError::Other(err) => err,
        other => anyhow::anyhow!("{other}"),
    })?;
    Ok(db)
}

/// Resolve a component's absolute on-disk directory from its
/// `path_segments[0].path` against the workspace `roots`. Same logic
/// as the private helper used by [`write_per_component_files`]: walk
/// roots in order, prefer one whose `<root>/<segment>` exists and
/// (if the entry has manifests) at least one manifest also resolves;
/// fall back to `roots[0]` joined with the segment when nothing
/// matches.
///
/// Exposed publicly so the report subcommands (`atlas modularity`)
/// can write per-component output under the same directory the
/// `atlas index` flow uses.
pub fn resolve_component_dir(
    segment_path: &Path,
    manifests: &[PathBuf],
    roots: &[PathBuf],
) -> PathBuf {
    if segment_path.is_absolute() {
        return segment_path.to_path_buf();
    }
    for root in roots {
        let abs = root.join(segment_path);
        if !abs.exists() {
            continue;
        }
        if manifests.is_empty() {
            return abs;
        }
        let any_manifest_present = manifests.iter().any(|m| root.join(m).exists());
        if any_manifest_present {
            return abs;
        }
    }
    roots
        .first()
        .map(|r| r.join(segment_path))
        .unwrap_or_else(|| segment_path.to_path_buf())
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

/// Persist the workspace root to `<output>/.atlas/config.yaml#roots`
/// (plan §4 PR-4 acceptance criterion). The file is otherwise
/// user-authored (operations / override_search are user knobs); we
/// load-or-default, overwrite only `roots`, and write back. The write
/// is via tempfile-then-rename so a crash mid-write cannot corrupt
/// the file.
fn persist_workspace_root(output_dir: &Path, root: &Path) -> Result<()> {
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
    existing.roots = vec![root.to_path_buf()];
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

/// Outcome of [`resolve_component_abs_dir`]: the absolute directory
/// for the component plus a boolean flag noting whether the resolution
/// fell back to `roots[0]` because no root passed the manifest /
/// existence check. The flag is exposed so the caller can emit a
/// warning, and so unit tests can assert the fallback fires when
/// expected.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedComponentDir {
    path: PathBuf,
    fell_back: bool,
}

/// Resolve a component's absolute on-disk directory from its
/// `path_segments[0].path` against the workspace `roots`. Disambiguation
/// via the entry's `manifests`:
///
/// - If `segment_path` is absolute, return it unchanged (no fallback).
/// - Otherwise, walk `roots` in order. A root matches when
///   `<root>/<segment_path>` exists AND (the entry has zero manifests
///   OR at least one of its manifests resolves to an existing file
///   under the root).
/// - If no root matches, fall back to `roots[0]` (or the segment_path
///   itself when `roots` is empty) and set `fell_back = true`.
///
/// `path_exists` abstracts the filesystem check so unit tests can
/// stub directory presence without materialising real files. The
/// production caller passes `Path::exists`.
fn resolve_component_abs_dir<F>(
    segment_path: &Path,
    manifests: &[PathBuf],
    roots: &[PathBuf],
    path_exists: F,
) -> ResolvedComponentDir
where
    F: Fn(&Path) -> bool,
{
    if segment_path.is_absolute() {
        return ResolvedComponentDir {
            path: segment_path.to_path_buf(),
            fell_back: false,
        };
    }

    for root in roots {
        let abs = root.join(segment_path);
        if !path_exists(&abs) {
            continue;
        }
        if manifests.is_empty() {
            return ResolvedComponentDir {
                path: abs,
                fell_back: false,
            };
        }
        let any_manifest_present = manifests.iter().any(|m| path_exists(&root.join(m)));
        if any_manifest_present {
            return ResolvedComponentDir {
                path: abs,
                fell_back: false,
            };
        }
    }

    let fallback = roots
        .first()
        .map(|r| r.join(segment_path))
        .unwrap_or_else(|| segment_path.to_path_buf());
    ResolvedComponentDir {
        path: fallback,
        fell_back: true,
    }
}

/// Walk every (non-deleted) component in `components_file` and emit
/// per-component `<component-path>/.atlas/cache/component.yaml` (PR-6 / PR-3) and
/// `<component-path>/.atlas/cache/surfaces.yaml` (PR-7 / PR-2) files. The
/// component's on-disk path is resolved by joining its
/// `path_segments[0].path` against the workspace root via
/// `resolve_component_abs_dir`. One `mkdir -p`
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
    gitignore_session: &mut GitignoreSession,
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
        // segment path is relative to the workspace root; probe for
        // the root whose `<root>/<segment>` actually contains the
        // component's manifests. Disambiguation via manifests guards
        // against components whose segment.path == "" (root-level
        // component) where the segment-dir check alone is insufficient.
        let resolution =
            resolve_component_abs_dir(&segment.path, &entry.manifests, roots, |p| p.exists());
        if resolution.fell_back {
            // Defensive fallback: every live component has manifests
            // today, so reaching this branch means either (a) the
            // entry has manifests but none resolve under any root
            // or (b) the entry has zero manifests AND no root
            // contains `<root>/<segment>`. Both shapes are
            // unexpected — emit a warning so the operator can see
            // the misrouting before files land in the wrong tree.
            eprintln!(
                "warning: component `{}` segment `{}` did not match any root via \
                 manifest existence; falling back to roots[0]. The per-component \
                 .atlas/ files may land under the wrong root.",
                entry.id.as_str(),
                segment.path.display()
            );
        }
        let candidate_abs = resolution.path;

        // PR-1 (Phase 3): ensure the per-component scope's
        // `.atlas/.gitignore` is in place before we materialise any
        // file under `<component>/.atlas/`. The session dedups so we
        // touch the file at most once per scope per session.
        gitignore_session.ensure(&candidate_abs);

        let target_dir = candidate_abs.join(".atlas");

        // -- cache/component.yaml (PR-6 / PR-3) ----------------------
        // Note: per_component_yaml_snapshot now consults
        // surfaces_yaml_snapshot for its fingerprint (PR-7), so
        // calling it transitively produces the surface artefacts
        // already. That call is cheap to repeat below thanks to
        // Salsa's memoisation of the underlying surface_of inputs.
        //
        // PR-3: the file now lives in the `cache/` sub-directory
        // (`<component>/.atlas/cache/component.yaml`). The
        // `write_yaml_atomic` helper performs `create_dir_all` on the
        // directory it receives, so we pass `target_dir.join("cache")`
        // rather than `target_dir`.
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

        let component_cache_dir = target_dir.join("cache");
        let component_file_path = component_cache_dir.join("component.yaml");
        if let Err(err) = write_per_component_atomic(
            &component_cache_dir,
            &component_file_path,
            &component_snapshot,
        ) {
            eprintln!(
                "warning: failed to write {}: {err:#}; the top-level components.yaml is unaffected",
                component_file_path.display()
            );
        }

        // -- cache/surfaces.yaml (PR-2 retrofit of PR-7) ----------------
        // Surfaces are projections too: a failed write is a non-fatal
        // warning. The top-level components.yaml does not (yet) carry
        // surface fingerprints, so a missing cache/surfaces.yaml degrades
        // L6 cache invalidation across components but does not break
        // the canonical output.
        //
        // PR-2 (Phase 3): path moved from the old per-component location
        // (directly in `.atlas/`) to `.atlas/cache/surfaces.yaml`.
        // Use `atomic_write` which creates parent directories (including
        // `cache/`) automatically.
        let surfaces_snapshot = match surfaces_yaml_snapshot(db, &entry.id) {
            Ok(arc) => arc,
            Err(err) => {
                eprintln!(
                    "warning: failed to project surfaces for `{}`: {err:#}; skipping per-component cache/surfaces.yaml write",
                    entry.id.as_str()
                );
                continue;
            }
        };

        let surfaces_file_path = target_dir.join("cache/surfaces.yaml");
        {
            // `atomic_write` requires serialised bytes; we serialise
            // explicitly here. `&*surfaces_snapshot` dereferences the Arc
            // so serde sees the concrete `SurfacesFile` impl.
            let yaml = match serde_yaml::to_string(&*surfaces_snapshot) {
                Ok(s) => s,
                Err(err) => {
                    eprintln!(
                        "warning: failed to serialise surfaces for `{}`: {err:#}; skipping write",
                        entry.id.as_str()
                    );
                    continue;
                }
            };
            if let Err(err) = atomic_write(&surfaces_file_path, yaml.as_bytes()) {
                eprintln!(
                    "warning: failed to write {}: {err:#}; the top-level components.yaml is unaffected",
                    surfaces_file_path.display()
                );
            }
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
/// Used by both the cache/component.yaml (PR-6 / PR-3) and surfaces.yaml (PR-7)
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

    // -----------------------------------------------------------------
    // resolve_component_abs_dir — PR-13 hangover bundle.
    // -----------------------------------------------------------------

    /// Build a closure that "exists" iff the candidate path matches one
    /// of the strings in `present`. Avoids touching the real filesystem
    /// for unit-level coverage of the resolution algorithm.
    fn exists_in(present: Vec<PathBuf>) -> impl Fn(&Path) -> bool {
        move |p: &Path| present.iter().any(|x| x == p)
    }

    #[test]
    fn resolve_component_abs_dir_picks_root_with_manifest_present() {
        let primary = PathBuf::from("/p/primary");
        let peer = PathBuf::from("/p/peer");
        let segment = PathBuf::from(""); // peer-root component, segment.path == ""
        let manifests = vec![PathBuf::from("Cargo.toml")];
        let roots = vec![primary.clone(), peer.clone()];

        // Both roots "contain" segment.path (each is itself a dir), but
        // only the peer carries `Cargo.toml`.
        let present = vec![primary.clone(), peer.clone(), peer.join("Cargo.toml")];
        let r = resolve_component_abs_dir(&segment, &manifests, &roots, exists_in(present));
        assert!(!r.fell_back);
        assert_eq!(r.path, peer);
    }

    #[test]
    fn resolve_component_abs_dir_falls_back_when_no_manifest_resolves() {
        let primary = PathBuf::from("/p/primary");
        let peer = PathBuf::from("/p/peer");
        let segment = PathBuf::from("");
        let manifests = vec![PathBuf::from("Cargo.toml")];
        let roots = vec![primary.clone(), peer.clone()];

        // Both root dirs exist but neither carries the manifest. The
        // function must signal `fell_back: true` (the warning trigger)
        // and choose `roots[0]`.
        let present = vec![primary.clone(), peer.clone()];
        let r = resolve_component_abs_dir(&segment, &manifests, &roots, exists_in(present));
        assert!(
            r.fell_back,
            "no manifest resolves under any root; the caller MUST emit \
             the defensive warning (fell_back must be true)"
        );
        assert_eq!(r.path, primary);
    }

    #[test]
    fn resolve_component_abs_dir_no_manifests_accepts_first_existing_root() {
        // Entries with zero manifests skip the manifest disambiguation;
        // the first root whose `<root>/<segment>` exists wins, no
        // fallback warning.
        let primary = PathBuf::from("/p/primary");
        let peer = PathBuf::from("/p/peer");
        let segment = PathBuf::from("crate-x");
        let roots = vec![primary.clone(), peer.clone()];

        let present = vec![peer.join("crate-x")]; // primary doesn't have it
        let r = resolve_component_abs_dir(&segment, &[], &roots, exists_in(present));
        assert!(!r.fell_back);
        assert_eq!(r.path, peer.join("crate-x"));
    }

    #[test]
    fn resolve_component_abs_dir_absolute_segment_short_circuits() {
        // Absolute segment paths bypass the roots walk entirely.
        let segment = PathBuf::from("/absolute/path");
        let roots = vec![PathBuf::from("/p/r1")];
        let r = resolve_component_abs_dir(&segment, &[], &roots, |_| false);
        assert!(!r.fell_back);
        assert_eq!(r.path, segment);
    }

    #[test]
    fn resolve_component_abs_dir_falls_back_when_no_root_contains_segment() {
        // The segment dir itself is missing under every root — fall
        // back to roots[0] and warn.
        let primary = PathBuf::from("/p/primary");
        let segment = PathBuf::from("nonexistent");
        let roots = vec![primary.clone()];
        let r = resolve_component_abs_dir(&segment, &[], &roots, exists_in(vec![]));
        assert!(r.fell_back);
        assert_eq!(r.path, primary.join("nonexistent"));
    }
}
