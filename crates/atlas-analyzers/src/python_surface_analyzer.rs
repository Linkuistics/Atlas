//! In-tree wrapper for the Python subprocess analyser (Atlas vNext
//! Phase 2 PR-3).
//!
//! Constructs a [`SubprocessAnalyzerSpec`] pointing at the
//! `python-analyzer` binary built by the
//! `crates/analyzers/python` workspace member, then wraps it in a
//! [`SubprocessAnalyzerProxy`]. The proxy's `binary_sha` flows into
//! the engine-side L5 fingerprint via tag `0x06` (PR-2's
//! contribution path) so a content change in the Python analyser
//! binary invalidates every L5 cache entry that consulted it.
//!
//! ## How the binary is located
//!
//! Cargo emits `CARGO_BIN_EXE_python-analyzer` for any test or
//! binary in this workspace that depends transitively on the
//! `[[bin]] name = "python-analyzer"` declared in
//! `crates/analyzers/python/Cargo.toml`. The path-dep arrow goes
//! `atlas-analyzers → atlas-python-analyzer (lib)` (so the lib's
//! types are available, the binary is a sibling artefact).
//!
//! For the *registry* construction in `AnalyzerRegistry::builtin`,
//! the path-dep alone does not guarantee the binary builds first
//! (cargo only orders bins after their owning crate's lib). PR-3
//! routes around this by computing the binary path at runtime: the
//! engine driver looks for the `python-analyzer` binary alongside
//! the currently-running test/binary's `target/<profile>` directory
//! and registers the proxy if found, or omits it when running
//! outside cargo (e.g. `atlas index .` invoked from `cargo install`
//! pre-Phase 3).
//!
//! ## Wire identity
//!
//! - `id`: [`atlas_python_analyzer::ANALYZER_ID`]
//!   (`"python-surface-analyzer"`).
//! - `version`: [`atlas_python_analyzer::ANALYZER_VERSION`]
//!   (`"1.0.0"`).
//! - `stage`: L5.
//! - `cost_class`: `DeterministicExpensive` (parser plus filesystem
//!   walk; deterministic but not as cheap as a single-file regex).
//! - Applicability: keyed on `pyproject.toml`, `setup.py`, or
//!   `requirements.txt` presence in the candidate dir's manifests
//!   plus the `python` language tag.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

use crate::subprocess::{SubprocessAnalyzerProxy, SubprocessAnalyzerSpec};
use crate::AnalyzerError;

/// Stable analyser id mirrored from the `atlas-python-analyzer`
/// library's constant. Kept in sync by the unit test
/// [`tests::python_id_string_matches_python_crate_constant`] in the
/// integration test layer (the dep arrow goes
/// `atlas-python-analyzer → atlas-analyzers`, so the python lib
/// can't be a build-time dep of this crate without a cycle; we
/// duplicate the constant and pin equality from the python crate's
/// own tests instead).
pub const PYTHON_ANALYZER_ID: &str = "python-surface-analyzer";

/// Free-form version paired with [`PYTHON_ANALYZER_ID`].
pub const PYTHON_ANALYZER_VERSION: &str = "1.0.0";

/// Default per-call timeout for the Python analyser (60 seconds,
/// matching the cross-analyser default in
/// [`crate::subprocess::process_pool`]).
pub const PYTHON_ANALYZER_DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Filename of the `python-analyzer` binary as cargo emits it (no
/// extension on Unix; `.exe` on Windows handled below).
const PYTHON_ANALYZER_BINARY_NAME: &str = "python-analyzer";

/// Locate the `python-analyzer` binary at runtime by walking up from
/// the currently-running executable's directory looking for a
/// sibling cargo target binary.
///
/// `cargo test --workspace` builds every workspace member's
/// `[[bin]]`s into `target/<profile>/`; the test executable itself
/// lives in `target/<profile>/deps/`. We walk one directory up to
/// reach the binary's location.
///
/// Returns `None` when the binary is not present (running outside a
/// cargo target tree, or against a workspace where the python
/// analyser wasn't built). On a `None` return the function emits a
/// `warning:` line on stderr listing every path it tried — silent
/// failure here would surface to operators as an empty Python surface
/// with no diagnostic, which is exactly the case PR-3 code-quality
/// review F-CQ-5 flagged.
pub fn locate_python_analyzer_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    // Walk up at most three levels searching for the sibling binary
    // — covers the `deps/` test-exe layout (one up) and any
    // additional nesting cargo introduces in the future. Record
    // every candidate path so the warning on a failed walk names
    // exactly where we looked.
    let mut candidate_paths: Vec<PathBuf> = Vec::with_capacity(3);
    for _ in 0..3 {
        let candidate = dir.join(format!(
            "{PYTHON_ANALYZER_BINARY_NAME}{}",
            std::env::consts::EXE_SUFFIX
        ));
        if candidate.is_file() {
            return Some(candidate);
        }
        candidate_paths.push(candidate);
        if !dir.pop() {
            break;
        }
    }
    eprintln!(
        "warning: python-analyzer binary not found in any of: {candidate_paths:?}. \
         Python components will produce empty surfaces. \
         (Build the analyser via `cargo build -p atlas-python-analyzer` or \
         install the workspace via `cargo install`.)",
    );
    None
}

/// Process-wide cache of [`SubprocessAnalyzerProxy`] instances keyed
/// on the binary path supplied to [`python_subprocess_spec`]. Build
/// the proxy once (the construction step hashes the binary content,
/// which is the per-call cost we are amortising) and re-use the
/// `Arc<SubprocessAnalyzerProxy>` for every subsequent
/// `surface_artefacts_of` call so a workspace with N Python
/// components incurs at most one binary hash + one process spawn,
/// not N. See PR-3 code-quality F-CQ-1.
///
/// The cache is Mutex-guarded but the locking is brief — only the
/// `HashMap::get` / `HashMap::insert` pair runs under the lock; the
/// proxy itself is shared via `Arc` so callers hold no lock during
/// `analyse`.
fn proxy_cache() -> &'static Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up (or construct) a process-wide cached
/// [`SubprocessAnalyzerProxy`] for the python-analyzer binary at
/// `spec.binary_path`. The first call hashes the binary and spawns
/// the proxy; subsequent calls keyed on the same path return the
/// existing `Arc`.
///
/// Errors mirror `SubprocessAnalyzerProxy::new`'s failure modes
/// (binary unreadable / unhashable). On error the cache is NOT
/// populated, so a transient failure can be retried by a later call
/// without leaving a poisoned entry.
///
/// Reusable shape: future subprocess analysers (e.g. C# / Elixir)
/// can follow the same pattern by calling this helper as long as
/// each analyser's binary path is distinct (the cache key is the
/// binary path, not the analyser id).
pub fn cached_subprocess_proxy(
    spec: SubprocessAnalyzerSpec,
) -> Result<Arc<SubprocessAnalyzerProxy>, AnalyzerError> {
    let cache = proxy_cache();
    {
        let guard = cache.lock().expect("python proxy cache mutex poisoned");
        if let Some(existing) = guard.get(&spec.binary_path) {
            return Ok(existing.clone());
        }
    }
    // Build outside the lock so a slow binary-hash on a large file
    // doesn't block other callers (e.g. parallel L8 map shards).
    let proxy = Arc::new(SubprocessAnalyzerProxy::new(spec.clone())?);
    let mut guard = cache.lock().expect("python proxy cache mutex poisoned");
    // Race: another caller may have inserted while we were building.
    // Their `Arc` wins; the duplicate proxy we just built is dropped.
    Ok(guard
        .entry(spec.binary_path.clone())
        .or_insert_with(|| proxy.clone())
        .clone())
}

/// Build the [`SubprocessAnalyzerSpec`] for the Python analyser.
///
/// `binary_path` is the on-disk path to the `python-analyzer` binary
/// produced by `cargo build` of the `crates/analyzers/python`
/// workspace member. The caller is responsible for resolving the
/// path (canonically via `env!("CARGO_BIN_EXE_python-analyzer")` in
/// tests, or via a runtime sibling-binary search in production).
///
/// Returns a spec ready to pass to
/// [`crate::AnalyzerRegistry::register_subprocess`] — or, more
/// commonly post-PR-3, into [`cached_subprocess_proxy`] so the engine
/// shares a single proxy across all Python components.
pub fn python_subprocess_spec(binary_path: PathBuf) -> SubprocessAnalyzerSpec {
    let command = vec![binary_path.to_string_lossy().into_owned()];
    SubprocessAnalyzerSpec {
        id: PYTHON_ANALYZER_ID.into(),
        version: PYTHON_ANALYZER_VERSION.into(),
        stage: Stage::L5,
        cost_class: CostClass::DeterministicExpensive,
        applicability: ApplicabilityPredicate {
            languages: vec!["python".into()],
            file_globs: vec![
                "**/pyproject.toml".into(),
                "**/setup.py".into(),
                "**/requirements.txt".into(),
            ],
            manifest_types: vec!["python".into()],
            ..Default::default()
        },
        command,
        binary_path,
        timeout: Some(Duration::from_secs(PYTHON_ANALYZER_DEFAULT_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn python_subprocess_spec_has_expected_identity() {
        let spec = python_subprocess_spec(PathBuf::from("/non/existent/python-analyzer"));
        assert_eq!(spec.id, PYTHON_ANALYZER_ID);
        assert_eq!(spec.version, PYTHON_ANALYZER_VERSION);
        assert_eq!(spec.stage, Stage::L5);
        assert_eq!(spec.cost_class, CostClass::DeterministicExpensive);
    }

    #[test]
    fn python_subprocess_spec_applicability_matches_python_signals() {
        let spec = python_subprocess_spec(PathBuf::from("/non/existent/python-analyzer"));
        assert!(spec.applicability.languages.contains(&"python".to_string()));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains("pyproject.toml")));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains("setup.py")));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains("requirements.txt")));
    }
}
