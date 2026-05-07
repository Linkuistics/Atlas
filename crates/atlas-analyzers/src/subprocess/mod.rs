//! Subprocess analyser transport (Atlas vNext, design §5.2 / §7.2).
//!
//! Wraps an out-of-process analyser binary in a
//! [`SubprocessAnalyzerProxy`] that implements
//! [`crate::Analyzer`] by speaking the stdio JSON wire protocol
//! defined in [`transport`] and [`wire_types`]. The protocol:
//!
//! 1. Parent spawns the child binary with stdin/stdout piped.
//! 2. Child writes a single [`handshake::Capabilities`] frame
//!    announcing its identity.
//! 3. Parent verifies the announced identity matches the
//!    registered [`SubprocessAnalyzerSpec`].
//! 4. For each subsequent dispatch, parent writes one
//!    [`wire_types::Request`] frame and reads one
//!    [`wire_types::Response`] frame.
//! 5. Pipeline shutdown drops the proxy; the child receives EOF
//!    on stdin, then `SIGTERM`, then (after a 5-second grace)
//!    `SIGKILL`.
//!
//! Subprocess analysers are dispatched through the same
//! [`crate::AnalyzerRegistry::dispatch`] machinery as in-process
//! analysers — the registry sees only the [`crate::Analyzer`]
//! trait.
//!
//! ## Phase 2 scope
//!
//! Only one subprocess per registered analyser; lifetime spans
//! pipeline run; no LLM access. See `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md`
//! §4 PR-2 for the full charter.

pub mod handshake;
pub mod process_pool;
pub mod transport;
pub mod wire_types;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::subprocess::handshake::Capabilities;
use crate::subprocess::process_pool::ProcessPool;
use crate::subprocess::wire_types::{
    Request, Response, WireFingerprintInput, WireTarget, WireTargetFile,
};
use crate::{
    AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, FingerprintInput, StageOutput, Target,
};

/// Declarative shape used to construct a [`SubprocessAnalyzerProxy`].
/// Mirrors the in-process analyser contract: the parent must know
/// id/version/stage/cost class up front so the registry's
/// dispatch order is well-defined.
#[derive(Debug, Clone)]
pub struct SubprocessAnalyzerSpec {
    /// Stable analyser id; matches the YAML form.
    pub id: String,
    /// Free-form version string.
    pub version: String,
    /// Stage the analyser plugs into.
    pub stage: Stage,
    /// Cost class.
    pub cost_class: CostClass,
    /// Applicability predicate the parent uses to short-circuit
    /// dispatch without crossing the process boundary.
    pub applicability: ApplicabilityPredicate,
    /// Argv of the analyser binary (`command[0]` is the binary,
    /// `command[1..]` the args). The parent resolves
    /// `command[0]` against `$PATH` (Phase 2) or against
    /// `override_search` (Phase 3+).
    pub command: Vec<String>,
    /// Path to the analyser binary on disk. Used to compute the
    /// binary content sha that flows into the cache fingerprint.
    pub binary_path: PathBuf,
    /// Per-call timeout. `None` falls back to
    /// [`process_pool::DEFAULT_TIMEOUT`].
    pub timeout: Option<Duration>,
}

/// Subprocess-backed [`Analyzer`] implementation. The proxy stores
/// no per-call state; every request goes through the
/// [`ProcessPool`].
pub struct SubprocessAnalyzerProxy {
    spec: SubprocessAnalyzerSpec,
    /// Lowercase 64-character sha256 hex of the binary at
    /// `spec.binary_path`, computed once at construction. Surfaced
    /// via [`SubprocessAnalyzerProxy::binary_sha`] so the engine
    /// can contribute it to L-stage fingerprints (tag `0x06`).
    binary_sha: String,
    pool: Arc<ProcessPool>,
}

impl std::fmt::Debug for SubprocessAnalyzerProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubprocessAnalyzerProxy")
            .field("id", &self.spec.id)
            .field("version", &self.spec.version)
            .field("stage", &self.spec.stage)
            .field("cost_class", &self.spec.cost_class)
            .field("binary_path", &self.spec.binary_path)
            .field("binary_sha", &self.binary_sha)
            .finish()
    }
}

impl SubprocessAnalyzerProxy {
    /// Construct a new proxy. Hashes `spec.binary_path` to compute
    /// the `binary_sha`. The child is NOT spawned here; the first
    /// dispatch performs the spawn + handshake.
    pub fn new(spec: SubprocessAnalyzerSpec) -> Result<Self, AnalyzerError> {
        let binary_sha = hash_binary(&spec.binary_path).map_err(|e| AnalyzerError::CallFailed {
            analyzer_id: spec.id.clone(),
            message: format!(
                "hashing analyser binary `{}` failed: {e}",
                spec.binary_path.display()
            ),
        })?;
        let expected_caps = Capabilities {
            id: spec.id.clone(),
            version: spec.version.clone(),
            stage: spec.stage,
            cost_class: spec.cost_class,
            applicability_predicate: spec.applicability.clone(),
        };
        let pool = Arc::new(ProcessPool::new(
            spec.command.clone(),
            expected_caps,
            spec.timeout,
            spec.binary_path.clone(),
        ));
        Ok(SubprocessAnalyzerProxy {
            spec,
            binary_sha,
            pool,
        })
    }

    /// The analyser binary's content sha (lowercase 64-char hex).
    /// Engine-side cache key construction calls this and contributes
    /// the value via `FingerprintBuilder::add_analyzer_binary_sha`.
    pub fn binary_sha(&self) -> &str {
        &self.binary_sha
    }
}

/// Output type the subprocess proxy emits. Holds the raw JSON
/// payload the child returned for an `analyse` request; the
/// downstream stage adapter is responsible for decoding it into a
/// stage-specific shape.
#[derive(Debug, Clone)]
pub struct SubprocessOutput {
    pub analyzer_id: String,
    pub payload: serde_json::Value,
}

crate::impl_stage_output!(SubprocessOutput);

impl Analyzer for SubprocessAnalyzerProxy {
    fn id(&self) -> &str {
        &self.spec.id
    }
    fn stage(&self) -> Stage {
        self.spec.stage
    }
    fn cost_class(&self) -> CostClass {
        self.spec.cost_class
    }
    fn version(&self) -> &str {
        &self.spec.version
    }

    fn applies(&self, target: &Target) -> bool {
        // Local short-circuit on the declared applicability —
        // matches what the parent asked the YAML to express. We
        // do NOT cross the process boundary for this; the child's
        // applicability predicate is structurally equal to the
        // parent's (the handshake verified it) so a duplicate
        // round-trip would be wasted I/O.
        applicability_matches(&self.spec.applicability, target)
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        // Round-trip into the child to ask it what inputs it
        // would consult. A failure here degrades to "no inputs":
        // the dispatcher's L3+ adapter then conservatively treats
        // the result as uncacheable. This matches the in-process
        // analyser contract — `fingerprint_inputs` is not allowed
        // to error in the trait, so we swallow.
        let req = Request::FingerprintInputs {
            target: target_to_wire(target),
        };
        match self.pool.call(&req) {
            Ok(Response::Confident { payload }) => {
                parse_fingerprint_inputs(&payload).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        let req = Request::Analyse {
            target: target_to_wire(target),
        };
        match self.pool.call(&req) {
            Ok(Response::Confident { payload }) => {
                AnalyzerResult::Confident(Box::new(SubprocessOutput {
                    analyzer_id: self.spec.id.clone(),
                    payload,
                }) as Box<dyn StageOutput>)
            }
            Ok(Response::Graded {
                payload,
                confidence,
            }) => AnalyzerResult::Graded {
                output: Box::new(SubprocessOutput {
                    analyzer_id: self.spec.id.clone(),
                    payload,
                }) as Box<dyn StageOutput>,
                confidence,
            },
            Ok(Response::Declines) => AnalyzerResult::Declines,
            Ok(Response::Error {
                message,
                error_kind,
            }) => {
                let err = if error_kind.as_deref() == Some("malformed_input") {
                    AnalyzerError::MalformedInput {
                        analyzer_id: self.spec.id.clone(),
                        message,
                    }
                } else {
                    AnalyzerError::CallFailed {
                        analyzer_id: self.spec.id.clone(),
                        message,
                    }
                };
                AnalyzerResult::Error(err)
            }
            Err(e) => AnalyzerResult::Error(e),
        }
    }
}

/// Cheap local copy of the in-process applicability check used by
/// [`crate::Analyzer::applies`]. The four shapes mirror the
/// `ApplicabilityPredicate` fields:
///
/// - `always: true` → unconditional.
/// - `file_globs` non-empty → at least one top-level file or
///   manifest path matches one of the patterns.
/// - `languages` non-empty → the target's language set
///   intersects this list.
/// - `manifest_types` non-empty → at least one pre-loaded
///   manifest's basename matches a known mapping (`Cargo.toml`
///   → `cargo`, `package.json` → `npm`, `pyproject.toml` /
///   `requirements.txt` → `python`, etc.).
///
/// Multiple non-empty fields combine with OR semantics; an
/// `always: false` predicate with no other field set never
/// applies.
fn applicability_matches(pred: &ApplicabilityPredicate, target: &Target) -> bool {
    if pred.always {
        return true;
    }
    if !pred.file_globs.is_empty() && file_globs_match(&pred.file_globs, target) {
        return true;
    }
    if !pred.languages.is_empty()
        && pred
            .languages
            .iter()
            .any(|l| target.languages.contains(l.as_str()))
    {
        return true;
    }
    if !pred.manifest_types.is_empty() && manifest_types_match(&pred.manifest_types, target) {
        return true;
    }
    false
}

/// Compile each glob with `globset` and check the candidate paths
/// (top-level files + manifest relpaths) for at least one match.
fn file_globs_match(globs: &[String], target: &Target) -> bool {
    use globset::{Glob, GlobSetBuilder};
    let mut builder = GlobSetBuilder::new();
    for g in globs {
        match Glob::new(g) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(_) => {
                // A malformed glob is dropped silently; the
                // dispatcher logs an `applies: false` either way.
                continue;
            }
        }
    }
    let set = match builder.build() {
        Ok(s) => s,
        Err(_) => return false,
    };
    for name in &target.top_level_files {
        if set.is_match(name) {
            return true;
        }
    }
    for m in &target.manifests {
        if set.is_match(&m.relpath) {
            return true;
        }
        if set.is_match(&m.name) {
            return true;
        }
    }
    false
}

fn manifest_types_match(types: &[String], target: &Target) -> bool {
    for m in &target.manifests {
        let kind = match m.name.as_str() {
            "Cargo.toml" => "cargo",
            "package.json" => "npm",
            "pyproject.toml" | "requirements.txt" => "python",
            "Dockerfile" => "docker",
            _ => continue,
        };
        if types.iter().any(|t| t == kind) {
            return true;
        }
    }
    false
}

fn target_to_wire(target: &Target) -> WireTarget {
    WireTarget {
        dir: target.dir.to_string_lossy().into_owned(),
        languages: target.languages.iter().cloned().collect(),
        manifests: target
            .manifests
            .iter()
            .map(|m| WireTargetFile {
                name: m.name.clone(),
                relpath: m.relpath.to_string_lossy().into_owned(),
                bytes_b64: BASE64.encode(&m.bytes),
                content_sha: m.content_sha.clone(),
            })
            .collect(),
        top_level_files: target.top_level_files.clone(),
    }
}

fn parse_fingerprint_inputs(payload: &serde_json::Value) -> Result<Vec<FingerprintInput>, String> {
    let arr = payload
        .as_array()
        .ok_or_else(|| "fingerprint_inputs payload is not an array".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let wired: WireFingerprintInput = serde_json::from_value(v.clone())
            .map_err(|e| format!("decoding WireFingerprintInput failed: {e}"))?;
        match wired {
            WireFingerprintInput::FileContentSha { sha } => {
                out.push(FingerprintInput::FileContentSha(sha));
            }
            WireFingerprintInput::Custom { tag, bytes_b64 } => {
                let bytes = BASE64
                    .decode(&bytes_b64)
                    .map_err(|e| format!("decoding base64 custom payload failed: {e}"))?;
                // Custom requires a `&'static str` tag; we leak the
                // decoded tag for the lifetime of the process.
                // Subprocess analysers should declare a small,
                // bounded set of tags so the leak is bounded.
                // TODO(phase3): replace `tag: &'static str` in
                // FingerprintInput::Custom with an owned
                // `Cow<'static, str>` so that novel tags from a
                // subprocess do not accumulate unbounded leaked
                // allocations. See PR-2 spec-review notes.
                let leaked: &'static str = Box::leak(tag.into_boxed_str());
                out.push(FingerprintInput::Custom { tag: leaked, bytes });
            }
        }
    }
    Ok(out)
}

/// Compute the lowercase 64-character sha256 hex of a binary on
/// disk. Used at proxy construction to produce `binary_sha`.
pub fn hash_binary(path: &Path) -> std::io::Result<String> {
    use std::fs::File;
    use std::io::{BufReader, Read};
    let f = File::open(path)?;
    let mut reader = BufReader::new(f);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut hex, "{b:02x}").expect("writing to String never fails");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::io::Write;

    #[test]
    fn hash_binary_is_64_char_lowercase_hex() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bin");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello-binary").unwrap();
        drop(f);
        let sha = hash_binary(&path).unwrap();
        assert_eq!(sha.len(), 64);
        assert!(sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn hash_binary_changes_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bin");
        std::fs::write(&path, b"v1").unwrap();
        let a = hash_binary(&path).unwrap();
        std::fs::write(&path, b"v2").unwrap();
        let b = hash_binary(&path).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn applicability_matches_always() {
        let pred = ApplicabilityPredicate {
            always: true,
            ..Default::default()
        };
        let target = Target {
            dir: "/x".into(),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        assert!(applicability_matches(&pred, &target));
    }

    #[test]
    fn applicability_matches_file_glob() {
        let pred = ApplicabilityPredicate {
            file_globs: vec!["**/Cargo.toml".into()],
            ..Default::default()
        };
        let target = Target {
            dir: "/x".into(),
            languages: BTreeSet::new(),
            manifests: vec![crate::TargetFile {
                name: "Cargo.toml".into(),
                relpath: "Cargo.toml".into(),
                bytes: Vec::new(),
                content_sha: "abc".into(),
            }],
            top_level_files: vec!["Cargo.toml".into()],
        };
        assert!(applicability_matches(&pred, &target));
    }

    #[test]
    fn applicability_matches_languages() {
        let pred = ApplicabilityPredicate {
            languages: vec!["rust".into()],
            ..Default::default()
        };
        let mut langs = BTreeSet::new();
        langs.insert("rust".to_string());
        let target = Target {
            dir: "/x".into(),
            languages: langs,
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        assert!(applicability_matches(&pred, &target));
    }

    #[test]
    fn applicability_returns_false_when_nothing_set() {
        let pred = ApplicabilityPredicate::default();
        let target = Target {
            dir: "/x".into(),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        assert!(!applicability_matches(&pred, &target));
    }

    #[test]
    fn target_to_wire_base64_encodes_bytes() {
        let target = Target {
            dir: "/ws/x".into(),
            languages: {
                let mut s = BTreeSet::new();
                s.insert("rust".into());
                s
            },
            manifests: vec![crate::TargetFile {
                name: "Cargo.toml".into(),
                relpath: "Cargo.toml".into(),
                bytes: b"[package]".to_vec(),
                content_sha: "abc".into(),
            }],
            top_level_files: vec!["Cargo.toml".into()],
        };
        let wired = target_to_wire(&target);
        assert_eq!(wired.dir, "/ws/x");
        assert_eq!(wired.languages, vec!["rust"]);
        assert_eq!(wired.manifests.len(), 1);
        let decoded = BASE64.decode(&wired.manifests[0].bytes_b64).unwrap();
        assert_eq!(decoded, b"[package]");
    }

    #[test]
    fn parse_fingerprint_inputs_handles_file_content_and_custom() {
        let payload = serde_json::json!([
            {"kind": "file_content_sha", "sha": "abc"},
            {"kind": "custom", "tag": "tx", "bytes_b64": BASE64.encode(b"hello")},
        ]);
        let parsed = parse_fingerprint_inputs(&payload).unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(matches!(parsed[0], FingerprintInput::FileContentSha(ref s) if s == "abc"));
        match &parsed[1] {
            FingerprintInput::Custom { tag, bytes } => {
                assert_eq!(*tag, "tx");
                assert_eq!(bytes, b"hello");
            }
            _ => panic!("expected Custom"),
        }
    }
}
