//! In-tree wrapper for the Racket subprocess analyser (Atlas vNext
//! Phase 2 PR-9).
//!
//! Constructs a [`SubprocessAnalyzerSpec`] pointing at the
//! `racket-analyzer` binary built by the
//! `crates/analyzers/racket` workspace member, then wraps it in a
//! [`SubprocessAnalyzerProxy`]. The proxy's `binary_sha` flows into
//! the engine-side L5 fingerprint via tag `0x06` (PR-2's contribution
//! path) so a content change in the Racket analyser binary invalidates
//! every L5 cache entry that consulted it.
//!
//! ## How the binary is located
//!
//! Cargo emits `CARGO_BIN_EXE_racket-analyzer` for any test or
//! binary in this workspace that depends transitively on the
//! `[[bin]] name = "racket-analyzer"` declared in
//! `crates/analyzers/racket/Cargo.toml`. Mirrors the Python analyser
//! pattern exactly.
//!
//! ## Wire identity
//!
//! - `id`: [`RACKET_ANALYZER_ID`] (`"racket-surface-analyzer"`).
//! - `version`: [`RACKET_ANALYZER_VERSION`] (`"1.0.0"`).
//! - `stage`: L5.
//! - `cost_class`: `DeterministicExpensive` (s-expression walk plus
//!   filesystem walk; deterministic but not trivially cheap).
//! - Applicability: keyed on `info.rkt` presence plus the `racket`
//!   language tag.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

use crate::python_surface_analyzer::cached_subprocess_proxy;
use crate::subprocess::{SubprocessAnalyzerProxy, SubprocessAnalyzerSpec};
use crate::AnalyzerError;

/// Stable analyser id mirrored from the `atlas-racket-analyzer`
/// library's constant. Duplicated here (without a build-dep on the
/// racket lib) using the same "pin equality via own-crate tests"
/// pattern as the Python analyser.
pub const RACKET_ANALYZER_ID: &str = "racket-surface-analyzer";

/// Free-form version paired with [`RACKET_ANALYZER_ID`].
pub const RACKET_ANALYZER_VERSION: &str = "1.0.0";

/// Default per-call timeout for the Racket analyser (60 seconds).
pub const RACKET_ANALYZER_DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Filename of the `racket-analyzer` binary as cargo emits it.
const RACKET_ANALYZER_BINARY_NAME: &str = "racket-analyzer";

/// Locate the `racket-analyzer` binary at runtime by walking up from
/// the currently-running executable's directory. Mirrors
/// [`crate::python_surface_analyzer::locate_python_analyzer_binary`]
/// exactly.
///
/// Returns `None` when the binary is not present (running outside a
/// cargo target tree, or against a workspace where the racket analyser
/// wasn't built). On a `None` return emits a warning to stderr.
pub fn locate_racket_analyzer_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    let mut candidate_paths: Vec<PathBuf> = Vec::with_capacity(3);
    for _ in 0..3 {
        let candidate = dir.join(format!(
            "{RACKET_ANALYZER_BINARY_NAME}{}",
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
        "warning: racket-analyzer binary not found in any of: {candidate_paths:?}. \
         Racket components will produce empty surfaces. \
         (Build the analyser via `cargo build -p atlas-racket-analyzer` or \
         install the workspace via `cargo install`.)",
    );
    None
}

/// Look up (or construct) a process-wide cached
/// [`SubprocessAnalyzerProxy`] for the racket-analyzer binary at
/// `spec.binary_path`.
///
/// Thin delegate to the shared
/// [`crate::python_surface_analyzer::cached_subprocess_proxy`] helper.
/// The shared helper is keyed on `spec.binary_path`, so racket and
/// python proxies share the same process-wide cache and the Wave-3
/// "reusable shape" invariant is maintained — no duplicate
/// `OnceLock`-backed cache for the racket path.
///
/// Errors mirror `SubprocessAnalyzerProxy::new`'s failure modes.
pub fn cached_racket_subprocess_proxy(
    spec: SubprocessAnalyzerSpec,
) -> Result<Arc<SubprocessAnalyzerProxy>, AnalyzerError> {
    cached_subprocess_proxy(spec)
}

/// Build the [`SubprocessAnalyzerSpec`] for the Racket analyser.
///
/// `binary_path` is the on-disk path to the `racket-analyzer` binary.
/// Mirrors [`crate::python_surface_analyzer::python_subprocess_spec`].
pub fn racket_subprocess_spec(binary_path: PathBuf) -> SubprocessAnalyzerSpec {
    let command = vec![binary_path.to_string_lossy().into_owned()];
    SubprocessAnalyzerSpec {
        id: RACKET_ANALYZER_ID.into(),
        version: RACKET_ANALYZER_VERSION.into(),
        stage: Stage::L5,
        cost_class: CostClass::DeterministicExpensive,
        applicability: ApplicabilityPredicate {
            languages: vec!["racket".into()],
            file_globs: vec!["**/info.rkt".into()],
            manifest_types: vec!["racket".into()],
            ..Default::default()
        },
        command,
        binary_path,
        timeout: Some(Duration::from_secs(RACKET_ANALYZER_DEFAULT_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn racket_subprocess_spec_has_expected_identity() {
        let spec = racket_subprocess_spec(PathBuf::from("/non/existent/racket-analyzer"));
        assert_eq!(spec.id, RACKET_ANALYZER_ID);
        assert_eq!(spec.version, RACKET_ANALYZER_VERSION);
        assert_eq!(spec.stage, Stage::L5);
        assert_eq!(spec.cost_class, CostClass::DeterministicExpensive);
    }

    #[test]
    fn racket_subprocess_spec_applicability_matches_racket_signals() {
        let spec = racket_subprocess_spec(PathBuf::from("/non/existent/racket-analyzer"));
        assert!(spec.applicability.languages.contains(&"racket".to_string()));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains("info.rkt")));
    }
}
