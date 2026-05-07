//! In-tree wrapper for the LispKit subprocess analyser (Atlas vNext
//! Phase 2 PR-10).
//!
//! Constructs a [`SubprocessAnalyzerSpec`] pointing at the
//! `lispkit-analyzer` binary built by the
//! `crates/analyzers/lispkit` workspace member, then wraps it in a
//! [`SubprocessAnalyzerProxy`]. The proxy's `binary_sha` flows into
//! the engine-side L5 fingerprint via tag `0x06` (PR-2's
//! contribution path) so a content change in the LispKit analyser
//! binary invalidates every L5 cache entry that consulted it.
//!
//! ## How the binary is located
//!
//! Cargo emits `CARGO_BIN_EXE_lispkit-analyzer` for any test or
//! binary in this workspace that depends transitively on the
//! `[[bin]] name = "lispkit-analyzer"` declared in
//! `crates/analyzers/lispkit/Cargo.toml`. The path-dep arrow goes
//! `atlas-analyzers → atlas-lispkit-analyzer (lib)` (so the lib's
//! types are available; the binary is a sibling artefact).
//!
//! For the *registry* construction in `AnalyzerRegistry::builtin`,
//! the path is computed at runtime: the engine driver looks for the
//! `lispkit-analyzer` binary alongside the currently-running
//! test/binary's `target/<profile>` directory and registers the
//! proxy if found, or omits it otherwise (same policy as the Python
//! analyser).
//!
//! ## Wire identity
//!
//! - `id`: [`LISPKIT_ANALYZER_ID`] (`"lispkit-surface-analyzer"`).
//! - `version`: [`LISPKIT_ANALYZER_VERSION`] (`"1.0.0"`).
//! - `stage`: L5.
//! - `cost_class`: `DeterministicExpensive`.
//! - Applicability: keyed on `**/*.sld` glob plus the `scheme` /
//!   `lispkit` language tags.
//!
//! ## Manifest convention
//!
//! LispKit components are identified by `*.sld` (Scheme Library
//! Definition) files — the R7RS-standard extension for
//! `define-library` forms. See `crates/analyzers/lispkit/src/lib.rs`
//! for the full rationale.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

use crate::subprocess::{SubprocessAnalyzerProxy, SubprocessAnalyzerSpec};
use crate::AnalyzerError;

/// Stable analyser id mirrored from the `atlas-lispkit-analyzer`
/// library's constant. Kept in sync by the unit test
/// [`tests::lispkit_id_string_matches_constant`] below; the dep arrow
/// goes `atlas-lispkit-analyzer → atlas-analyzers` so the lispkit lib
/// can't be a build-time dep of this crate without a cycle — we
/// duplicate the constant and pin equality from the lispkit crate's
/// own tests.
pub const LISPKIT_ANALYZER_ID: &str = "lispkit-surface-analyzer";

/// Free-form version paired with [`LISPKIT_ANALYZER_ID`].
pub const LISPKIT_ANALYZER_VERSION: &str = "1.0.0";

/// Default per-call timeout for the LispKit analyser (60 seconds,
/// matching the cross-analyser default).
pub const LISPKIT_ANALYZER_DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Filename of the `lispkit-analyzer` binary as cargo emits it.
const LISPKIT_ANALYZER_BINARY_NAME: &str = "lispkit-analyzer";

/// Locate the `lispkit-analyzer` binary at runtime by walking up from
/// the currently-running executable's directory looking for a sibling
/// cargo target binary. Returns `None` when the binary is not present
/// (running outside a cargo target tree or against a workspace where
/// the lispkit analyser wasn't built). On `None`, emits a `warning:`
/// line on stderr listing every path tried.
pub fn locate_lispkit_analyzer_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    let mut candidate_paths: Vec<PathBuf> = Vec::with_capacity(3);
    for _ in 0..3 {
        let candidate = dir.join(format!(
            "{LISPKIT_ANALYZER_BINARY_NAME}{}",
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
        "warning: lispkit-analyzer binary not found in any of: {candidate_paths:?}. \
         LispKit components will produce empty surfaces. \
         (Build the analyser via `cargo build -p atlas-lispkit-analyzer` or \
         install the workspace via `cargo install`.)",
    );
    None
}

/// Process-wide cache of [`SubprocessAnalyzerProxy`] instances keyed
/// on the binary path supplied to [`lispkit_subprocess_spec`]. Build
/// the proxy once (the construction step hashes the binary content)
/// and re-use the `Arc<SubprocessAnalyzerProxy>` for every subsequent
/// call. Follows the same pattern as
/// [`crate::python_surface_analyzer::cached_subprocess_proxy`] (PR-3
/// F-CQ-1).
fn lispkit_proxy_cache() -> &'static Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up (or construct) a process-wide cached
/// [`SubprocessAnalyzerProxy`] for the lispkit-analyzer binary at
/// `spec.binary_path`. The first call hashes the binary and spawns
/// the proxy; subsequent calls keyed on the same path return the
/// existing `Arc`.
pub fn cached_lispkit_subprocess_proxy(
    spec: SubprocessAnalyzerSpec,
) -> Result<Arc<SubprocessAnalyzerProxy>, AnalyzerError> {
    let cache = lispkit_proxy_cache();
    {
        let guard = cache.lock().expect("lispkit proxy cache mutex poisoned");
        if let Some(existing) = guard.get(&spec.binary_path) {
            return Ok(existing.clone());
        }
    }
    let proxy = Arc::new(SubprocessAnalyzerProxy::new(spec.clone())?);
    let mut guard = cache.lock().expect("lispkit proxy cache mutex poisoned");
    Ok(guard
        .entry(spec.binary_path.clone())
        .or_insert_with(|| proxy.clone())
        .clone())
}

/// Build the [`SubprocessAnalyzerSpec`] for the LispKit analyser.
///
/// `binary_path` is the on-disk path to the `lispkit-analyzer` binary
/// produced by `cargo build` of the `crates/analyzers/lispkit`
/// workspace member. The caller resolves the path canonically via
/// `env!("CARGO_BIN_EXE_lispkit-analyzer")` in tests, or via
/// [`locate_lispkit_analyzer_binary`] in production.
pub fn lispkit_subprocess_spec(binary_path: PathBuf) -> SubprocessAnalyzerSpec {
    let command = vec![binary_path.to_string_lossy().into_owned()];
    SubprocessAnalyzerSpec {
        id: LISPKIT_ANALYZER_ID.into(),
        version: LISPKIT_ANALYZER_VERSION.into(),
        stage: Stage::L5,
        cost_class: CostClass::DeterministicExpensive,
        applicability: ApplicabilityPredicate {
            languages: vec!["scheme".into(), "lispkit".into()],
            file_globs: vec!["**/*.sld".into()],
            manifest_types: vec!["lispkit".into()],
            ..Default::default()
        },
        command,
        binary_path,
        timeout: Some(Duration::from_secs(LISPKIT_ANALYZER_DEFAULT_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn lispkit_subprocess_spec_has_expected_identity() {
        let spec = lispkit_subprocess_spec(PathBuf::from("/non/existent/lispkit-analyzer"));
        assert_eq!(spec.id, LISPKIT_ANALYZER_ID);
        assert_eq!(spec.version, LISPKIT_ANALYZER_VERSION);
        assert_eq!(spec.stage, Stage::L5);
        assert_eq!(spec.cost_class, CostClass::DeterministicExpensive);
    }

    #[test]
    fn lispkit_subprocess_spec_applicability_matches_sld_signal() {
        let spec = lispkit_subprocess_spec(PathBuf::from("/non/existent/lispkit-analyzer"));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains(".sld")));
        assert!(spec
            .applicability
            .languages
            .iter()
            .any(|l| l == "scheme" || l == "lispkit"));
    }

    #[test]
    fn lispkit_id_string_matches_constant() {
        // Drift guard: if the constant value changes, this test fails
        // and reminds the author to update the lispkit crate constant too.
        assert_eq!(LISPKIT_ANALYZER_ID, "lispkit-surface-analyzer");
        assert_eq!(LISPKIT_ANALYZER_VERSION, "1.0.0");
    }
}
