//! In-tree wrapper for the Elixir subprocess analyser (Atlas vNext
//! Phase 2 PR-8).
//!
//! Constructs a [`SubprocessAnalyzerSpec`] pointing at the
//! `elixir-analyzer` binary built by the
//! `crates/analyzers/elixir` workspace member, then wraps it in a
//! [`SubprocessAnalyzerProxy`]. The proxy's `binary_sha` flows into
//! the engine-side L5 fingerprint via tag `0x06` (PR-2's
//! contribution path) so a content change in the Elixir analyser
//! binary invalidates every L5 cache entry that consulted it.
//!
//! ## How the binary is located
//!
//! Cargo emits `CARGO_BIN_EXE_elixir-analyzer` for any test or
//! binary in this workspace that depends transitively on the
//! `[[bin]] name = "elixir-analyzer"` declared in
//! `crates/analyzers/elixir/Cargo.toml`. The path-dep arrow goes
//! `atlas-analyzers → atlas-elixir-analyzer (lib)`.
//!
//! For the *registry* construction, the path-dep alone does not
//! guarantee the binary builds first. The engine driver looks for
//! the `elixir-analyzer` binary alongside the currently-running
//! test/binary's `target/<profile>` directory and registers the
//! proxy if found, or omits it when running outside cargo.
//!
//! ## Wire identity
//!
//! - `id`: [`ELIXIR_ANALYZER_ID`] (`"elixir-surface-analyzer"`).
//! - `version`: [`ELIXIR_ANALYZER_VERSION`] (`"1.0.0"`).
//! - `stage`: L5.
//! - `cost_class`: `DeterministicExpensive`.
//! - Applicability: keyed on `mix.exs` presence + `elixir` language tag.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

use crate::subprocess::{SubprocessAnalyzerProxy, SubprocessAnalyzerSpec};
use crate::AnalyzerError;

/// Stable analyser id mirrored from the `atlas-elixir-analyzer`
/// library's constant. Kept in sync by the integration tests.
pub const ELIXIR_ANALYZER_ID: &str = "elixir-surface-analyzer";

/// Free-form version paired with [`ELIXIR_ANALYZER_ID`].
pub const ELIXIR_ANALYZER_VERSION: &str = "1.0.0";

/// Default per-call timeout for the Elixir analyser (60 seconds,
/// matching the cross-analyser default).
pub const ELIXIR_ANALYZER_DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Filename of the `elixir-analyzer` binary as cargo emits it.
const ELIXIR_ANALYZER_BINARY_NAME: &str = "elixir-analyzer";

/// Locate the `elixir-analyzer` binary at runtime by walking up from
/// the currently-running executable's directory.
///
/// Returns `None` when the binary is not present (running outside a
/// cargo target tree, or against a workspace where the elixir
/// analyser wasn't built). On `None` the function emits a `warning:`
/// on stderr naming every path it tried.
pub fn locate_elixir_analyzer_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    let mut candidate_paths: Vec<PathBuf> = Vec::with_capacity(3);
    for _ in 0..3 {
        let candidate = dir.join(format!(
            "{ELIXIR_ANALYZER_BINARY_NAME}{}",
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
        "warning: elixir-analyzer binary not found in any of: {candidate_paths:?}. \
         Elixir components will produce empty surfaces. \
         (Build the analyser via `cargo build -p atlas-elixir-analyzer` or \
         install the workspace via `cargo install`.)",
    );
    None
}

/// Process-wide cache of [`SubprocessAnalyzerProxy`] instances keyed
/// on the binary path supplied to [`elixir_subprocess_spec`]. Mirrors
/// the Python-side proxy cache to amortise binary-hash + process-spawn
/// cost across all Elixir components in a workspace.
fn proxy_cache() -> &'static Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<SubprocessAnalyzerProxy>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up (or construct) a process-wide cached
/// [`SubprocessAnalyzerProxy`] for the elixir-analyzer binary at
/// `spec.binary_path`. The first call hashes the binary and spawns
/// the proxy; subsequent calls return the existing `Arc`.
///
/// Errors mirror `SubprocessAnalyzerProxy::new`'s failure modes.
pub fn cached_elixir_subprocess_proxy(
    spec: SubprocessAnalyzerSpec,
) -> Result<Arc<SubprocessAnalyzerProxy>, AnalyzerError> {
    let cache = proxy_cache();
    {
        let guard = cache.lock().expect("elixir proxy cache mutex poisoned");
        if let Some(existing) = guard.get(&spec.binary_path) {
            return Ok(existing.clone());
        }
    }
    let proxy = Arc::new(SubprocessAnalyzerProxy::new(spec.clone())?);
    let mut guard = cache.lock().expect("elixir proxy cache mutex poisoned");
    Ok(guard
        .entry(spec.binary_path.clone())
        .or_insert_with(|| proxy.clone())
        .clone())
}

/// Build the [`SubprocessAnalyzerSpec`] for the Elixir analyser.
pub fn elixir_subprocess_spec(binary_path: PathBuf) -> SubprocessAnalyzerSpec {
    let command = vec![binary_path.to_string_lossy().into_owned()];
    SubprocessAnalyzerSpec {
        id: ELIXIR_ANALYZER_ID.into(),
        version: ELIXIR_ANALYZER_VERSION.into(),
        stage: Stage::L5,
        cost_class: CostClass::DeterministicExpensive,
        applicability: ApplicabilityPredicate {
            languages: vec!["elixir".into()],
            file_globs: vec!["**/mix.exs".into()],
            manifest_types: vec!["elixir".into()],
            ..Default::default()
        },
        command,
        binary_path,
        timeout: Some(Duration::from_secs(ELIXIR_ANALYZER_DEFAULT_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn elixir_subprocess_spec_has_expected_identity() {
        let spec = elixir_subprocess_spec(PathBuf::from("/non/existent/elixir-analyzer"));
        assert_eq!(spec.id, ELIXIR_ANALYZER_ID);
        assert_eq!(spec.version, ELIXIR_ANALYZER_VERSION);
        assert_eq!(spec.stage, Stage::L5);
        assert_eq!(spec.cost_class, CostClass::DeterministicExpensive);
    }

    #[test]
    fn elixir_subprocess_spec_applicability_matches_elixir_signals() {
        let spec = elixir_subprocess_spec(PathBuf::from("/non/existent/elixir-analyzer"));
        assert!(spec.applicability.languages.contains(&"elixir".to_string()));
        assert!(spec
            .applicability
            .file_globs
            .iter()
            .any(|g| g.contains("mix.exs")));
    }
}
