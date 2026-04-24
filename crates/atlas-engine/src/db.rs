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

use atlas_index::{ComponentsFile, ExternalsFile, OverridesFile, RelatedComponentsFile};
use atlas_llm::{LlmBackend, LlmFingerprint};
use salsa::Setter;

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
    pub llm_fingerprint: Arc<LlmFingerprint>,
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
}

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
            Arc::new(fingerprint),
        );
        db.workspace = Some(workspace);
        db
    }

    pub fn backend(&self) -> &Arc<dyn LlmBackend> {
        &self.backend
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

    pub fn set_llm_fingerprint(&mut self, value: LlmFingerprint) {
        let ws = self.workspace();
        if **ws.llm_fingerprint(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_llm_fingerprint(self).to(Arc::new(value));
    }

    pub fn file_by_path(&self, path: &Path) -> Option<File> {
        self.files_by_path.lock().unwrap().get(path).copied()
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
