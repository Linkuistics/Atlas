//! In-tree wrapper for the C# subprocess analyser (Atlas vNext
//! Phase 2 PR-6).
//!
//! Constructs a [`SubprocessAnalyzerSpec`] pointing at the
//! `csharp-analyzer` binary built by the `crates/analyzers/csharp`
//! workspace member, then wraps it in a [`SubprocessAnalyzerProxy`].
//! The proxy's `binary_sha` flows into the engine-side L5 fingerprint
//! via tag `0x06` so a content change in the C# analyser binary
//! invalidates every L5 cache entry that consulted it.
//!
//! ## How the binary is located
//!
//! Cargo emits `CARGO_BIN_EXE_csharp-analyzer` for any test or binary
//! in this workspace that depends transitively on the `[[bin]] name =
//! "csharp-analyzer"` declared in `crates/analyzers/csharp/Cargo.toml`.
//!
//! For the *registry* construction the engine locates the binary at
//! runtime by walking up from the current executable's directory, the
//! same pattern as [`crate::python_surface_analyzer`].
//!
//! ## Wire identity
//!
//! - `id`: [`CSHARP_ANALYZER_ID`] (`"csharp-surface-analyzer"`).
//! - `version`: [`CSHARP_ANALYZER_VERSION`] (`"1.0.0"`).
//! - `stage`: L5.
//! - `cost_class`: `DeterministicExpensive`.
//! - Applicability: keyed on `*.csproj` / `*.sln` manifests plus the
//!   `csharp` language tag.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

use crate::subprocess::{SubprocessAnalyzerProxy, SubprocessAnalyzerSpec};
use crate::AnalyzerError;

/// Stable analyser id. Kept in sync with the constant inside
/// `atlas_csharp_analyzer::ANALYZER_ID` via the unit test
/// [`tests::csharp_id_and_version_constants_match_expected_values`].
pub const CSHARP_ANALYZER_ID: &str = "csharp-surface-analyzer";

/// Free-form version paired with [`CSHARP_ANALYZER_ID`].
pub const CSHARP_ANALYZER_VERSION: &str = "1.0.0";

/// Default per-call timeout for the C# analyser (60 seconds).
pub const CSHARP_ANALYZER_DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Filename of the `csharp-analyzer` binary.
const CSHARP_ANALYZER_BINARY_NAME: &str = "csharp-analyzer";

/// Locate the `csharp-analyzer` binary at runtime by walking up from
/// the currently-running executable's directory. Mirrors
/// [`crate::python_surface_analyzer::locate_python_analyzer_binary`].
///
/// Returns `None` when the binary is not present, emitting a `warning:`
/// to stderr listing every path tried.
pub fn locate_csharp_analyzer_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    let mut candidate_paths: Vec<PathBuf> = Vec::with_capacity(3);
    for _ in 0..3 {
        let candidate = dir.join(format!(
            "{CSHARP_ANALYZER_BINARY_NAME}{}",
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
        "warning: csharp-analyzer binary not found in any of: {candidate_paths:?}. \
         C# components will produce empty surfaces. \
         (Build the analyser via `cargo build -p atlas-csharp-analyzer` or \
         install the workspace via `cargo install`.)",
    );
    None
}

/// Process-wide cache of [`SubprocessAnalyzerProxy`] instances keyed
/// on the binary path. Mirrors the Python analyser's `proxy_cache`
/// pattern (PR-3 code-quality F-CQ-1) so a workspace with N C#
/// components incurs at most one binary hash + one process spawn.
fn proxy_cache() -> &'static Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up (or construct) a process-wide cached
/// [`SubprocessAnalyzerProxy`] for the csharp-analyzer binary at
/// `spec.binary_path`.
///
/// Mirrors [`crate::python_surface_analyzer::cached_subprocess_proxy`].
pub fn cached_csharp_subprocess_proxy(
    spec: SubprocessAnalyzerSpec,
) -> Result<Arc<SubprocessAnalyzerProxy>, AnalyzerError> {
    let cache = proxy_cache();
    {
        let guard = cache.lock().expect("csharp proxy cache mutex poisoned");
        if let Some(existing) = guard.get(&spec.binary_path) {
            return Ok(existing.clone());
        }
    }
    let proxy = Arc::new(SubprocessAnalyzerProxy::new(spec.clone())?);
    let mut guard = cache.lock().expect("csharp proxy cache mutex poisoned");
    Ok(guard
        .entry(spec.binary_path.clone())
        .or_insert_with(|| proxy.clone())
        .clone())
}

/// Build the [`SubprocessAnalyzerSpec`] for the C# analyser.
pub fn csharp_subprocess_spec(binary_path: PathBuf) -> SubprocessAnalyzerSpec {
    let command = vec![binary_path.to_string_lossy().into_owned()];
    SubprocessAnalyzerSpec {
        id: CSHARP_ANALYZER_ID.into(),
        version: CSHARP_ANALYZER_VERSION.into(),
        stage: Stage::L5,
        cost_class: CostClass::DeterministicExpensive,
        applicability: ApplicabilityPredicate {
            languages: vec!["csharp".into()],
            file_globs: vec!["**/*.csproj".into(), "**/*.sln".into()],
            manifest_types: vec!["csharp".into()],
            ..Default::default()
        },
        command,
        binary_path,
        timeout: Some(Duration::from_secs(CSHARP_ANALYZER_DEFAULT_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csharp_subprocess_spec_has_expected_identity() {
        let spec = csharp_subprocess_spec(PathBuf::from("/non/existent/csharp-analyzer"));
        assert_eq!(spec.id, CSHARP_ANALYZER_ID);
        assert_eq!(spec.version, CSHARP_ANALYZER_VERSION);
        assert_eq!(spec.stage, Stage::L5);
        assert_eq!(spec.cost_class, CostClass::DeterministicExpensive);
    }

    #[test]
    fn csharp_subprocess_spec_applicability_matches_csharp_signals() {
        let spec = csharp_subprocess_spec(PathBuf::from("/non/existent/csharp-analyzer"));
        assert!(spec.applicability.languages.contains(&"csharp".to_string()));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains(".csproj")));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains(".sln")));
    }

    #[test]
    fn csharp_id_and_version_constants_match_expected_values() {
        assert_eq!(CSHARP_ANALYZER_ID, "csharp-surface-analyzer");
        assert_eq!(CSHARP_ANALYZER_VERSION, "1.0.0");
    }
}
