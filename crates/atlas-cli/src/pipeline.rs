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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use atlas_engine::{
    components_yaml_snapshot_with_prompt_shas, external_components_yaml_snapshot,
    related_components_yaml_snapshot, run_fixedpoint, seed_filesystem_excluding, AtlasDatabase,
    FixedpointConfig, Phase, ProgressEvent, ProgressSink,
};
use atlas_index::{
    load_or_default_components, load_or_default_externals, load_or_default_overrides,
    load_or_default_related_components, load_or_default_subsystems,
    load_or_default_subsystems_overrides, save_components_atomic, save_externals_atomic,
    save_related_components_atomic, save_subsystems_atomic, ComponentsFile, OverridesFile,
    SubsystemsFile, SubsystemsOverridesFile,
};
use atlas_llm::{LlmBackend, LlmFingerprint, TokenCounter};

use crate::backend::BudgetSentinel;
use crate::cache_io;
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
        let subsystems_overrides =
            load_or_default_subsystems_overrides(&subsystems_overrides_path)
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

    let mut db = AtlasDatabase::new(backend.clone(), config.root.clone(), fingerprint.clone());
    let cache_path = config.output_dir.join("llm-cache.json");
    cache_io::load_into(&cache_path, db.llm_cache());
    seed_filesystem_excluding(
        &mut db,
        &config.root,
        &config.output_dir,
        config.respect_gitignore,
    )
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
    let n = live_components.len() as u64;
    for (i, comp) in live_components.iter().enumerate() {
        reporter.on_event(ProgressEvent::Surface {
            component_id: comp.id.clone(),
            relpath: atlas_engine::relpath_of(comp),
            k: (i as u64) + 1,
            n,
        });
        let _ = atlas_engine::surface_of(&db, comp.id.clone());
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

        // Persist the LLM response cache. A failed save is not fatal —
        // the outputs are already committed; we just lose the cache hit
        // on the next run, which is a perf regression, not a
        // correctness issue.
        let _ = cache_io::save_from(&cache_path, db.llm_cache());
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
