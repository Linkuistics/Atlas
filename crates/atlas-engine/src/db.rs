//! The Atlas Salsa database, together with the L0 input structs
//! [`File`] and [`Workspace`] that seed every downstream query.
//!
//! # Shape
//!
//! There is one [`Workspace`] input per database (the handle is created
//! in [`AtlasDatabase::new`]) holding:
//!
//! - The filesystem root.
//! - The list of registered [`File`] inputs (each carries a path and
//!   its byte contents).
//! - The list of directories that contain a `.git` marker (a separate
//!   field because `.git` directories are typically not registered as
//!   regular files).
//! - The four prior-run YAML snapshots plus the LLM fingerprint,
//!   wrapped in [`std::sync::Arc`] so the foreign types in `atlas-index`
//!   and `atlas-llm` do not need to grow a Salsa `Update` dependency.
//!
//! Per-file granularity lets a content change to a single file
//! invalidate only queries that actually read that file, without
//! rebuilding the file list itself. Adding or removing files changes
//! the `files` vector and is correctly propagated.
//!
//! `backend` and `files_by_path` live outside the Salsa storage so the
//! database can hand out a cheap `File` handle from a path (used during
//! seeding and in tests) without racing Salsa's own state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use atlas_index::{
    ComponentsFile, ExternalsFile, OverridesFile, RelatedComponentsFile, SubsystemsOverridesFile,
};
use atlas_llm::{LlmBackend, LlmFingerprint};
use salsa::Setter;

use crate::llm_cache::LlmResponseCache;

/// One file known to the engine. Salsa input: content changes via
/// [`File::set_bytes`] invalidate queries that read the file, without
/// disturbing queries that only looked at the enclosing [`Workspace`]'s
/// file list.
#[salsa::input(debug)]
pub struct File {
    #[returns(ref)]
    pub path: PathBuf,
    #[returns(ref)]
    pub bytes: Arc<Vec<u8>>,
}

/// Run-wide L0 inputs. A single `Workspace` handle is created in
/// [`AtlasDatabase::new`] and read by every downstream query.
#[salsa::input(debug)]
pub struct Workspace {
    #[returns(ref)]
    pub root: PathBuf,
    #[returns(ref)]
    pub files: Vec<File>,
    /// Directories that contain a `.git` marker (as directory or file
    /// — the latter covers submodules and worktrees). Stored
    /// independently because `.git` contents are not ordinarily
    /// registered as [`File`]s.
    #[returns(ref)]
    pub git_boundary_dirs: Vec<PathBuf>,
    #[returns(ref)]
    pub prior_components: Arc<ComponentsFile>,
    #[returns(ref)]
    pub prior_externals: Arc<ExternalsFile>,
    #[returns(ref)]
    pub prior_related_components: Arc<RelatedComponentsFile>,
    #[returns(ref)]
    pub components_overrides: Arc<OverridesFile>,
    #[returns(ref)]
    pub subsystems_overrides: Arc<SubsystemsOverridesFile>,
    #[returns(ref)]
    pub llm_fingerprint: Arc<LlmFingerprint>,
    /// Fixedpoint back-edge: per-component sub-directories that L8 has
    /// decided to treat as new L2 candidate roots. L2 reads this to
    /// expand its `candidate_dirs` set during within-run recursion; the
    /// driver in [`crate::fixedpoint`] mutates it between iterations.
    /// Empty on a fresh run (no back-edge fired yet).
    #[returns(ref)]
    pub carve_back_edge: Arc<BTreeMap<String, Vec<PathBuf>>>,
}

/// Event-log entry recorded when Salsa executes (not cache-hits) a
/// tracked query, exposed via [`AtlasDatabase::enable_execution_log`]
/// for cache-behaviour tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedEvent {
    pub description: String,
}

#[salsa::db]
#[derive(Clone)]
pub struct AtlasDatabase {
    storage: salsa::Storage<Self>,
    backend: Arc<dyn LlmBackend>,
    /// `None` only during the narrow window inside [`AtlasDatabase::new`]
    /// between allocating the database struct and creating the
    /// [`Workspace`] input that every query keys off. Every public
    /// method sees `Some`.
    workspace: Option<Workspace>,
    files_by_path: Arc<Mutex<BTreeMap<PathBuf, File>>>,
    execution_log: Arc<Mutex<Option<Vec<ExecutedEvent>>>>,
    /// Per-run LLM response cache consulted by L5/L6 (and any future
    /// LLM-driven query) via [`AtlasDatabase::call_llm_cached`]. Keyed
    /// by backend fingerprint + request, so a Workspace-input edit that
    /// does not move the request-level inputs is a cache hit.
    llm_cache: LlmResponseCache,
    /// Maximum recursion depth for L8's sub-carve policy. 0 means "only
    /// top-level components". Default [`DEFAULT_MAX_DEPTH`] matches the
    /// design-doc §8.2 fixedpoint cap. Lives outside Salsa storage
    /// because L8 is a plain function over `&AtlasDatabase`.
    max_depth: Arc<Mutex<u32>>,
    /// Iteration counter bumped by [`crate::fixedpoint::run_fixedpoint`]
    /// on each back-edge pass. Zero before the driver runs; reset
    /// explicitly when a caller wants a fresh count.
    fixedpoint_iterations: Arc<Mutex<u32>>,
    /// Bound on parallel `is_component` calls in L8's map step. Lives
    /// outside Salsa storage for the same reason as `max_depth`. A value
    /// of 1 forces serial execution (preserving the pre-rayon path).
    map_concurrency: Arc<Mutex<usize>>,
}

/// Default value for [`AtlasDatabase::max_depth`]. Mirrors design §8.2.
pub const DEFAULT_MAX_DEPTH: u32 = 8;

/// Default bound on parallel L8 map calls. 8 matches the spec's
/// "start with 8 in flight" recommendation and is conservative
/// against Anthropic / OpenAI per-account rate limits while still
/// delivering a meaningful speedup over serial execution.
pub const DEFAULT_MAP_CONCURRENCY: usize = 8;

impl AtlasDatabase {
    /// Construct a database seeded with an LLM backend, a root path,
    /// and an initial LLM fingerprint. Prior-run YAMLs default to
    /// empty; callers that have them on hand install them with
    /// [`AtlasDatabase::set_prior_components`] and friends.
    pub fn new(backend: Arc<dyn LlmBackend>, root: PathBuf, fingerprint: LlmFingerprint) -> Self {
        let execution_log: Arc<Mutex<Option<Vec<ExecutedEvent>>>> = Arc::default();
        let storage = salsa::Storage::new(Some(Box::new({
            let execution_log = execution_log.clone();
            move |event: salsa::Event| {
                if let salsa::EventKind::WillExecute { .. } = event.kind {
                    if let Some(log) = &mut *execution_log.lock().unwrap() {
                        log.push(ExecutedEvent {
                            description: format!("{:?}", event.kind),
                        });
                    }
                }
            }
        })));

        // `Workspace::new` needs `&dyn salsa::Database`, so build the
        // database first with `workspace: None`, then install the
        // input handle.
        let mut db = AtlasDatabase {
            storage,
            backend,
            workspace: None,
            files_by_path: Arc::default(),
            execution_log,
            llm_cache: LlmResponseCache::new(),
            max_depth: Arc::new(Mutex::new(DEFAULT_MAX_DEPTH)),
            fixedpoint_iterations: Arc::new(Mutex::new(0)),
            map_concurrency: Arc::new(Mutex::new(DEFAULT_MAP_CONCURRENCY)),
        };
        let workspace = Workspace::new(
            &db,
            root,
            Vec::new(),
            Vec::new(),
            Arc::new(ComponentsFile::default()),
            Arc::new(ExternalsFile::default()),
            Arc::new(RelatedComponentsFile::default()),
            Arc::new(OverridesFile::default()),
            Arc::new(SubsystemsOverridesFile::default()),
            Arc::new(fingerprint),
            Arc::new(BTreeMap::new()),
        );
        db.workspace = Some(workspace);
        db
    }

    pub fn backend(&self) -> &Arc<dyn LlmBackend> {
        &self.backend
    }

    /// Memoised backend call. Every LLM-adjacent query (L3's
    /// `classify_via_llm`, L5 `surface_of`, L6 `candidate_edges_for`)
    /// should go through this wrapper so the cache-hit contract in the
    /// task 9 exit criteria holds.
    pub fn call_llm_cached(
        &self,
        request: &atlas_llm::LlmRequest,
    ) -> Result<Arc<serde_json::Value>, atlas_llm::LlmError> {
        self.llm_cache.call_cached(self.backend.as_ref(), request)
    }

    pub fn llm_cache(&self) -> &LlmResponseCache {
        &self.llm_cache
    }

    pub fn workspace(&self) -> Workspace {
        self.workspace
            .expect("workspace is always installed by AtlasDatabase::new")
    }

    /// Register a file at `path` with the given bytes, returning the
    /// newly-created [`File`] input. If the path was already
    /// registered, its bytes are updated in place (and only if
    /// actually different) and the existing handle is returned. The
    /// "only if different" check matters: Salsa input setters
    /// unconditionally bump the revision, so calling `set_bytes` on
    /// an unchanged file would invalidate every query that read its
    /// bytes, breaking the cache-hit-on-no-op contract.
    pub fn register_file(&mut self, path: PathBuf, bytes: Vec<u8>) -> File {
        let bytes = Arc::new(bytes);
        let existing = self.files_by_path.lock().unwrap().get(&path).copied();
        if let Some(file) = existing {
            let current_matches = {
                let current: &Arc<Vec<u8>> = file.bytes(self as &dyn salsa::Database);
                current.as_ref() == bytes.as_ref()
            };
            if !current_matches {
                file.set_bytes(self).to(bytes);
            }
            file
        } else {
            let file = File::new(self, path.clone(), bytes);
            self.files_by_path.lock().unwrap().insert(path, file);
            file
        }
    }

    /// Install the full file set on the [`Workspace`] input, skipping
    /// the write when the new list equals the currently-installed one
    /// — otherwise the unchanged-reseed path would needlessly
    /// invalidate every query that reads `workspace.files`.
    pub fn set_workspace_files(&mut self, files: Vec<File>) {
        let ws = self.workspace();
        if ws.files(self as &dyn salsa::Database) == &files {
            return;
        }
        ws.set_files(self).to(files);
    }

    pub fn set_git_boundary_dirs(&mut self, dirs: Vec<PathBuf>) {
        let ws = self.workspace();
        if ws.git_boundary_dirs(self as &dyn salsa::Database) == &dirs {
            return;
        }
        ws.set_git_boundary_dirs(self).to(dirs);
    }

    pub fn set_prior_components(&mut self, value: ComponentsFile) {
        let ws = self.workspace();
        if **ws.prior_components(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_prior_components(self).to(Arc::new(value));
    }

    pub fn set_prior_externals(&mut self, value: ExternalsFile) {
        let ws = self.workspace();
        if **ws.prior_externals(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_prior_externals(self).to(Arc::new(value));
    }

    pub fn set_prior_related_components(&mut self, value: RelatedComponentsFile) {
        let ws = self.workspace();
        if **ws.prior_related_components(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_prior_related_components(self).to(Arc::new(value));
    }

    pub fn set_components_overrides(&mut self, value: OverridesFile) {
        let ws = self.workspace();
        if **ws.components_overrides(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_components_overrides(self).to(Arc::new(value));
    }

    pub fn set_subsystems_overrides(&mut self, value: SubsystemsOverridesFile) {
        let ws = self.workspace();
        if **ws.subsystems_overrides(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_subsystems_overrides(self).to(Arc::new(value));
    }

    pub fn set_llm_fingerprint(&mut self, value: LlmFingerprint) {
        let ws = self.workspace();
        if **ws.llm_fingerprint(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_llm_fingerprint(self).to(Arc::new(value));
    }

    pub fn set_carve_back_edge(&mut self, value: BTreeMap<String, Vec<PathBuf>>) {
        let ws = self.workspace();
        if **ws.carve_back_edge(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_carve_back_edge(self).to(Arc::new(value));
    }

    pub fn file_by_path(&self, path: &Path) -> Option<File> {
        self.files_by_path.lock().unwrap().get(path).copied()
    }

    pub fn max_depth(&self) -> u32 {
        *self.max_depth.lock().expect("max_depth poisoned")
    }

    pub fn set_max_depth(&self, value: u32) {
        *self.max_depth.lock().expect("max_depth poisoned") = value;
    }

    /// Bound on parallel `is_component` calls in L8's map step.
    /// 0 is normalised to 1 — nothing else clamps the value.
    pub fn map_concurrency(&self) -> usize {
        let raw = *self
            .map_concurrency
            .lock()
            .expect("map_concurrency poisoned");
        raw.max(1)
    }

    pub fn set_map_concurrency(&self, value: usize) {
        *self
            .map_concurrency
            .lock()
            .expect("map_concurrency poisoned") = value.max(1);
    }

    /// Number of fixedpoint iterations recorded by the most recent
    /// driver run. Readable without a database mutation.
    pub fn fixedpoint_iteration_count(&self) -> u32 {
        *self
            .fixedpoint_iterations
            .lock()
            .expect("fixedpoint_iterations poisoned")
    }

    /// Overwrite the fixedpoint iteration counter. Used by
    /// [`crate::fixedpoint::run_fixedpoint`] to record progress;
    /// exposed publicly so tests can reset the counter.
    pub fn set_fixedpoint_iteration_count(&self, value: u32) {
        *self
            .fixedpoint_iterations
            .lock()
            .expect("fixedpoint_iterations poisoned") = value;
    }

    /// Enable recording of Salsa `WillExecute` events until the next
    /// [`AtlasDatabase::take_execution_log`] call. Tests use this to
    /// assert that a query was re-run (or skipped) after an input
    /// change.
    pub fn enable_execution_log(&self) {
        let mut log = self.execution_log.lock().unwrap();
        if log.is_none() {
            *log = Some(Vec::new());
        }
    }

    pub fn take_execution_log(&self) -> Vec<ExecutedEvent> {
        let mut log = self.execution_log.lock().unwrap();
        log.as_mut().map(std::mem::take).unwrap_or_default()
    }
}

#[salsa::db]
impl salsa::Database for AtlasDatabase {}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_index::SubsystemsOverridesFile;
    use atlas_llm::TestBackend;

    fn fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: "test-backend".into(),
            backend_version: "v0".into(),
        }
    }

    #[test]
    fn set_subsystems_overrides_skip_if_equal_preserves_arc_identity() {
        let backend: Arc<dyn LlmBackend> = Arc::new(TestBackend::new());
        let mut db = AtlasDatabase::new(backend, std::path::PathBuf::from("/tmp/x"), fingerprint());
        let initial = SubsystemsOverridesFile::default();
        db.set_subsystems_overrides(initial.clone());
        let ws = db.workspace();
        let arc1 = ws.subsystems_overrides(&db as &dyn salsa::Database).clone();
        db.set_subsystems_overrides(initial);
        let arc2 = ws.subsystems_overrides(&db as &dyn salsa::Database).clone();
        assert!(
            Arc::ptr_eq(&arc1, &arc2),
            "skip-if-equal must keep the same Arc allocation"
        );
    }
}
