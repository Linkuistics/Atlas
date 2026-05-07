//! In-tree wrapper for the Dart subprocess analyser (Atlas vNext Phase 2 PR-7).
//!
//! Constructs a [`SubprocessAnalyzerSpec`] pointing at the `dart-analyzer`
//! binary built by the `crates/analyzers/dart` workspace member, then wraps
//! it in a [`SubprocessAnalyzerProxy`]. The proxy's `binary_sha` flows into
//! the engine-side L5 fingerprint via tag `0x06` (PR-2's contribution path)
//! so a content change in the Dart analyser binary invalidates every L5 cache
//! entry that consulted it.
//!
//! ## Pattern
//!
//! Mirrors [`crate::python_surface_analyzer`] exactly. See that module's
//! docstring for the design rationale; all decisions made there apply here.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

use crate::subprocess::{SubprocessAnalyzerProxy, SubprocessAnalyzerSpec};
use crate::AnalyzerError;

/// Stable analyser id. Mirrored from the `atlas-dart-analyzer` library's
/// constant; kept in sync via the unit test in that crate.
pub const DART_ANALYZER_ID: &str = "dart-surface-analyzer";

/// Free-form version paired with [`DART_ANALYZER_ID`].
pub const DART_ANALYZER_VERSION: &str = "1.0.0";

/// Default per-call timeout for the Dart analyser (60 seconds).
pub const DART_ANALYZER_DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Filename of the `dart-analyzer` binary as Cargo emits it.
const DART_ANALYZER_BINARY_NAME: &str = "dart-analyzer";

/// Locate the `dart-analyzer` binary at runtime by walking up from the
/// currently-running executable's directory looking for a sibling Cargo
/// target binary.
///
/// Mirrors [`crate::python_surface_analyzer::locate_python_analyzer_binary`].
pub fn locate_dart_analyzer_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    let mut candidate_paths: Vec<PathBuf> = Vec::with_capacity(3);
    for _ in 0..3 {
        let candidate = dir.join(format!(
            "{DART_ANALYZER_BINARY_NAME}{}",
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
        "warning: dart-analyzer binary not found in any of: {candidate_paths:?}. \
         Dart components will produce empty surfaces. \
         (Build the analyser via `cargo build -p atlas-dart-analyzer` or \
         install the workspace via `cargo install`.)",
    );
    None
}

/// Process-wide cache of [`SubprocessAnalyzerProxy`] instances keyed on
/// the binary path supplied to [`dart_subprocess_spec`].
///
/// Mirrors [`crate::python_surface_analyzer::cached_subprocess_proxy`] —
/// see that module's docstring for the amortisation rationale.
fn dart_proxy_cache() -> &'static Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up (or construct) a process-wide cached
/// [`SubprocessAnalyzerProxy`] for the dart-analyzer binary.
///
/// Mirrors [`crate::python_surface_analyzer::cached_subprocess_proxy`].
pub fn cached_dart_subprocess_proxy(
    spec: SubprocessAnalyzerSpec,
) -> Result<Arc<SubprocessAnalyzerProxy>, AnalyzerError> {
    let cache = dart_proxy_cache();
    {
        let guard = cache.lock().expect("dart proxy cache mutex poisoned");
        if let Some(existing) = guard.get(&spec.binary_path) {
            return Ok(existing.clone());
        }
    }
    let proxy = Arc::new(SubprocessAnalyzerProxy::new(spec.clone())?);
    let mut guard = cache.lock().expect("dart proxy cache mutex poisoned");
    Ok(guard
        .entry(spec.binary_path.clone())
        .or_insert_with(|| proxy.clone())
        .clone())
}

/// Build the [`SubprocessAnalyzerSpec`] for the Dart analyser.
///
/// `binary_path` is the on-disk path to the `dart-analyzer` binary. The
/// caller resolves it via `env!("CARGO_BIN_EXE_dart-analyzer")` in tests
/// or via [`locate_dart_analyzer_binary`] at runtime.
pub fn dart_subprocess_spec(binary_path: PathBuf) -> SubprocessAnalyzerSpec {
    let command = vec![binary_path.to_string_lossy().into_owned()];
    SubprocessAnalyzerSpec {
        id: DART_ANALYZER_ID.into(),
        version: DART_ANALYZER_VERSION.into(),
        stage: Stage::L5,
        cost_class: CostClass::DeterministicExpensive,
        applicability: ApplicabilityPredicate {
            languages: vec!["dart".into()],
            file_globs: vec!["**/pubspec.yaml".into()],
            manifest_types: vec!["dart".into()],
            ..Default::default()
        },
        command,
        binary_path,
        timeout: Some(Duration::from_secs(DART_ANALYZER_DEFAULT_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn dart_subprocess_spec_has_expected_identity() {
        let spec = dart_subprocess_spec(PathBuf::from("/non/existent/dart-analyzer"));
        assert_eq!(spec.id, DART_ANALYZER_ID);
        assert_eq!(spec.version, DART_ANALYZER_VERSION);
        assert_eq!(spec.stage, Stage::L5);
        assert_eq!(spec.cost_class, CostClass::DeterministicExpensive);
    }

    #[test]
    fn dart_subprocess_spec_applicability_matches_dart_signals() {
        let spec = dart_subprocess_spec(PathBuf::from("/non/existent/dart-analyzer"));
        assert!(spec.applicability.languages.contains(&"dart".to_string()));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains("pubspec.yaml")));
    }
}
