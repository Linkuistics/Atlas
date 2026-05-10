//! LLM-classify fallback analyser.
//!
//! Wraps an opaque [`LlmHook`] supplied by the engine. The analyser
//! has the lowest precedence in the L3 dispatch order
//! (`CostClass::LlmCheap`, `Confidence::Declines`): the dispatcher
//! consults it only after every deterministic classifier has either
//! `Declines`'d or returned `Confident`. The hook itself decides
//! which model to call; the analyser just packages the candidate
//! evidence and parses the response.
//!
//! Phase 1 design §6.6 example calls this analyser
//! `llm-classify-fallback`. The id is part of the cache fingerprint
//! so a future model swap (handled by the LLM fingerprint, not by an
//! id rename) does not silently reuse stale results.
//!
//! For PR-5 we wire only the analyser shape — the engine adapter
//! decides whether to install the hook. The L3 adapter in
//! `atlas-engine` defaults to installing one that calls
//! `db.call_llm_cached(...)` so existing v1 behaviour is preserved.

use std::sync::Arc;

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, FingerprintInput, Target};

/// Analyser id. Stable across versions; matches the `id` field in
/// the design §6.6 example.
pub const ANALYZER_ID: &str = "llm-classify-fallback";

/// Analyser version string. Bump when the prompt-builder shape or
/// the JSON-response parser changes meaningfully — that change
/// invalidates the L3 stage cache.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Errors the LLM hook can raise. Mapped onto
/// [`AnalyzerError::CallFailed`] inside the analyser.
#[derive(Debug, thiserror::Error)]
pub enum LlmHookError {
    #[error("LLM backend setup failed: {0}")]
    Setup(String),
    #[error("LLM backend call failed: {0}")]
    Call(String),
}

/// LLM call hook the engine injects into [`AnalysisContext`]. The
/// analyser provides a raw classification request payload (a JSON
/// object with the candidate evidence); the hook returns the parsed
/// response payload. Hook implementations are expected to:
///
/// 1. Render the engine's `classify` prompt with the supplied inputs.
/// 2. Route through the in-memory LLM cache (per design §5.4).
/// 3. Surface the response as a `serde_json::Value`.
///
/// The hook trait stays small (one method) so the engine can install
/// any backend without leaking Salsa types into this crate.
///
/// `Send` only — not `Sync` — because the hook may close over a
/// `!Sync` `AtlasDatabase` handle (Salsa's `ZalsaLocal` is `!Sync`).
/// The hook is constructed inside `is_component` and invoked from
/// the same thread, so the relaxed bound matches actual usage. The
/// rayon-parallel L8 map step clones a fresh `AtlasDatabase` per
/// worker before constructing its own hook, so the per-thread
/// invariant is preserved.
pub trait LlmHook: Send {
    /// Execute one classify call with the supplied evidence payload.
    /// The payload schema matches the keys the `classify.md` prompt
    /// template expects (see `l3_classify::build_llm_inputs`).
    fn classify(&self, inputs: &serde_json::Value) -> Result<Arc<serde_json::Value>, LlmHookError>;
}

/// Confidence value emitted alongside the parsed response. Mirrors
/// the `evidence_grade` field the LLM returns; mapped onto a
/// `[0.0, 1.0]` float.
fn confidence_from_grade(grade: &str) -> f32 {
    match grade {
        "strong" => 0.9,
        "medium" => 0.6,
        "weak" => 0.3,
        _ => 0.5,
    }
}

/// Output produced by [`LlmClassifyAnalyzer::analyse`]. The engine's
/// L3 adapter receives this through `Box<dyn StageOutput>` and
/// downcasts to recover the parsed JSON. PR-5 keeps the parser
/// adapter inside `atlas-engine` (it owns the v1
/// `parse_llm_response` logic); the analyser produces the raw value
/// only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmClassifyOutput {
    /// Parsed JSON object the LLM returned. The L3 adapter walks the
    /// fields onto `Classification`.
    pub response: serde_json::Value,
}

/// Inputs the engine prepared for the classify prompt. The analyser
/// owns the call but not the prompt construction — the engine builds
/// `inputs` from its rationale bundle and hands it through here.
///
/// Phase 1 keeps the field intentionally untyped (`Value`) because
/// the prompt-builder lives in `atlas-engine` and this crate must
/// not depend on that direction. PR-7 will tighten the shape when
/// the L5 analyser wraps a similar hook.
#[derive(Debug, Clone)]
pub struct ClassifyEvidence {
    pub inputs: serde_json::Value,
}

/// Analyser implementation. Construct one per registry; the analyser
/// is stateless beyond holding the evidence builder closure that the
/// engine attaches to each [`Target`] (out of band — this struct has
/// no fields).
#[derive(Debug, Default)]
pub struct LlmClassifyAnalyzer;

impl LlmClassifyAnalyzer {
    pub fn new() -> Self {
        LlmClassifyAnalyzer
    }
}

impl Analyzer for LlmClassifyAnalyzer {
    fn id(&self) -> &str {
        ANALYZER_ID
    }
    fn stage(&self) -> Stage {
        Stage::L3
    }
    fn cost_class(&self) -> CostClass {
        CostClass::LlmCheap
    }
    fn version(&self) -> &str {
        ANALYZER_VERSION
    }

    /// Always applicable — the LLM analyser is the registry's
    /// fallback. The dispatcher reaches it only after every cheaper
    /// applicable analyser declined.
    fn applies(&self, _target: &Target) -> bool {
        true
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        // The LLM call is keyed off the candidate's manifest contents
        // (already sha'd by the engine) and the candidate dir's path.
        // The dispatcher will additionally contribute the `LlmFingerprint`
        // and the prompt sha through the `FingerprintBuilder`'s
        // dedicated channels — this analyser only declares its own
        // inputs.
        let mut out = Vec::with_capacity(target.manifests.len() + 1);
        for m in &target.manifests {
            out.push(FingerprintInput::FileContentSha(m.content_sha.clone()));
        }
        out.push(FingerprintInput::Custom {
            tag: "candidate-dir",
            bytes: target.dir.to_string_lossy().as_bytes().to_vec(),
        });
        out
    }

    fn analyse(&self, ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        let Some(hook) = ctx.llm.as_ref() else {
            // No LLM hook installed — the analyser cannot run. Decline
            // so the dispatcher records "no applicable analyser
            // produced an answer"; the L3 adapter then falls back to
            // its weak-unknown classification.
            return AnalyzerResult::Declines;
        };
        // The evidence is carried on `Target.manifests` (the engine
        // pre-loads it) plus the dir path. The richer per-prompt JSON
        // (rationale bundle, doc headings, manifest contents map) is
        // built by the engine adapter, NOT by the analyser, and
        // installed on the hook itself — this struct is a thin pass-
        // through. The `inputs` payload here is just the bytes the
        // hook expects to find.
        //
        // Phase 1: the engine's `LlmHook` impl ignores `inputs` and
        // rebuilds the full prompt JSON from the candidate dir
        // because the v1 `build_llm_inputs` lives in `atlas-engine`
        // and the analyser cannot reach it from this crate. The
        // `inputs` payload is therefore reserved for Phase 2 once the
        // analyser owns the prompt.
        let inputs = serde_json::json!({
            "dir": target.dir.to_string_lossy(),
        });
        match hook.classify(&inputs) {
            Ok(value) => {
                let confidence = (*value)
                    .get("evidence_grade")
                    .and_then(|v| v.as_str())
                    .map(confidence_from_grade)
                    .unwrap_or(0.5);
                let output = LlmClassifyOutput {
                    response: (*value).clone(),
                };
                AnalyzerResult::Graded {
                    output: Box::new(output),
                    confidence,
                }
            }
            Err(e) => AnalyzerResult::Error(AnalyzerError::CallFailed {
                analyzer_id: ANALYZER_ID.to_string(),
                message: e.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Mutex;

    struct StubHook {
        responses: Mutex<Vec<serde_json::Value>>,
        calls: Mutex<u32>,
    }

    impl StubHook {
        fn new(responses: Vec<serde_json::Value>) -> Self {
            StubHook {
                responses: Mutex::new(responses),
                calls: Mutex::new(0),
            }
        }
    }

    impl LlmHook for StubHook {
        fn classify(
            &self,
            _inputs: &serde_json::Value,
        ) -> Result<Arc<serde_json::Value>, LlmHookError> {
            *self.calls.lock().unwrap() += 1;
            let mut q = self.responses.lock().unwrap();
            if q.is_empty() {
                return Err(LlmHookError::Call("no canned response".into()));
            }
            Ok(Arc::new(q.remove(0)))
        }
    }

    fn target() -> Target {
        Target {
            dir: PathBuf::from("/ws/some-dir"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        }
    }

    #[test]
    fn declines_when_no_llm_hook_is_installed() {
        let an = LlmClassifyAnalyzer::new();
        let ctx = AnalysisContext::deterministic_only();
        let r = an.analyse(&ctx, &target());
        assert!(matches!(r, AnalyzerResult::Declines), "got {r:?}");
    }

    #[test]
    fn graded_with_confidence_from_evidence_grade() {
        let hook = Arc::new(StubHook::new(vec![serde_json::json!({
            "kind": "rust-library",
            "evidence_grade": "strong",
            "rationale": "ok",
        })]));
        let ctx = AnalysisContext::with_llm(hook.clone());
        let an = LlmClassifyAnalyzer::new();
        let r = an.analyse(&ctx, &target());
        match r {
            AnalyzerResult::Graded { confidence, output } => {
                assert!((confidence - 0.9).abs() < 1e-4, "got {confidence}");
                let parsed = output
                    .as_any()
                    .downcast_ref::<LlmClassifyOutput>()
                    .expect("output must be LlmClassifyOutput");
                assert_eq!(parsed.response["kind"], "rust-library");
            }
            other => panic!("expected Graded, got {other:?}"),
        }
        assert_eq!(*hook.calls.lock().unwrap(), 1);
    }

    #[test]
    fn errors_when_hook_returns_err() {
        let hook = Arc::new(StubHook::new(Vec::new())); // no canned response → error
        let ctx = AnalysisContext::with_llm(hook);
        let an = LlmClassifyAnalyzer::new();
        let r = an.analyse(&ctx, &target());
        assert!(matches!(r, AnalyzerResult::Error(_)), "got {r:?}");
    }

    #[test]
    fn applies_is_unconditional() {
        let an = LlmClassifyAnalyzer::new();
        assert!(an.applies(&target()));
    }

    #[test]
    fn fingerprint_inputs_include_dir_and_manifest_shas() {
        let mut t = target();
        t.manifests.push(crate::TargetFile {
            name: "Cargo.toml".into(),
            relpath: PathBuf::from("Cargo.toml"),
            bytes: b"[package]".to_vec(),
            content_sha: "abc".into(),
        });
        let inputs = LlmClassifyAnalyzer::new().fingerprint_inputs(&t);
        // Always at least the dir custom contribution.
        assert!(!inputs.is_empty());
        // Two calls with the same Target produce the same set.
        let again = LlmClassifyAnalyzer::new().fingerprint_inputs(&t);
        assert_eq!(inputs, again);
    }
}
