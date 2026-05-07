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

use std::path::PathBuf;
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

use crate::subprocess::SubprocessAnalyzerSpec;

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
/// analyser wasn't built).
pub fn locate_python_analyzer_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    // Walk up at most three levels searching for the sibling binary
    // — covers the `deps/` test-exe layout (one up) and any
    // additional nesting cargo introduces in the future.
    for _ in 0..3 {
        let candidate = dir.join(format!(
            "{PYTHON_ANALYZER_BINARY_NAME}{}",
            std::env::consts::EXE_SUFFIX
        ));
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
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
/// [`crate::AnalyzerRegistry::register_subprocess`].
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
