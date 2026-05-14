//! Shared test fixtures for the engine and downstream crates.
//!
//! This module is gated behind `cfg(any(test, feature = "test-fixtures"))`
//! so that release builds of `atlas-engine` carry no test-only symbols.
//! Downstream test binaries (e.g. `atlas-cli`'s integration tests)
//! enable the `test-fixtures` feature on their dev-dependency entry to
//! pull this module in.
//!
//! The canonical fixture provided here is [`LenientBackend`] — a
//! permissive in-memory `LlmBackend` that returns canned default
//! responses for every prompt. It exists so tests that don't care
//! about LLM behaviour can drive the full pipeline without network
//! access while still exercising the `LlmBackend` invariants
//! (deterministic output, stable fingerprint).
//!
//! Tests that need to assert on backend invocation patterns can read
//! [`LenientBackend::calls`] / [`LenientBackend::call_count`].
//! Tests that need a non-Rust default classification (e.g. the
//! polyglot L5 surface tests) construct via
//! [`LenientBackend::with_classify`].
//!
//! Tests that need to inject `LlmError`s for specific prompts (e.g.
//! the L5 surface tests' `ClassifyCountingBackend`) are not served by
//! this fixture and should keep their bespoke counting/erroring
//! backends inline — `LenientBackend` is intentionally infallible.

use std::sync::{Arc, Mutex};

use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::{json, Value};

/// Permissive canned-response `LlmBackend` used by integration tests.
///
/// Every call is logged so tests can assert on which prompts fired.
/// The default classification is a Rust library; tests that need a
/// different default construct via [`LenientBackend::with_classify`].
///
/// The struct is intentionally not `Clone` — clone the `Arc<Self>`
/// returned by the constructors instead.
pub struct LenientBackend {
    fingerprint: LlmFingerprint,
    classify_response: Value,
    stage1_surface_response: Value,
    call_log: Mutex<Vec<(PromptId, String)>>,
}

impl LenientBackend {
    /// Construct a backend with the default Rust-library classify
    /// response and a stub Stage1 surface.
    ///
    /// Tests that need a different default classification (e.g. the
    /// C# / Python / Elixir polyglot surface tests) should use
    /// [`Self::with_classify`].
    pub fn new(fingerprint: LlmFingerprint) -> Arc<Self> {
        Arc::new(Self {
            fingerprint,
            classify_response: default_rust_library_classify(),
            stage1_surface_response: default_stage1_surface(),
            call_log: Mutex::new(Vec::new()),
        })
    }

    /// Construct a backend with a caller-supplied default
    /// `Classify` response. Useful for polyglot tests where the
    /// default `kind` / `language` need to match the fixture.
    pub fn with_classify(fingerprint: LlmFingerprint, classify_response: Value) -> Arc<Self> {
        Arc::new(Self {
            fingerprint,
            classify_response,
            stage1_surface_response: default_stage1_surface(),
            call_log: Mutex::new(Vec::new()),
        })
    }

    /// Return the prompt ids of every call this backend has serviced,
    /// in invocation order. Suitable for `.iter().filter(...)` checks.
    pub fn calls(&self) -> Vec<PromptId> {
        self.call_log
            .lock()
            .expect("call_log mutex poisoned")
            .iter()
            .map(|(p, _)| *p)
            .collect()
    }

    /// Return the full call log including the canonical-string form
    /// of each request's `inputs`. Tests that need to distinguish
    /// per-component calls (e.g. persistent-cache tests) use this.
    pub fn calls_with_inputs(&self) -> Vec<(PromptId, String)> {
        self.call_log
            .lock()
            .expect("call_log mutex poisoned")
            .clone()
    }

    /// Total number of calls serviced.
    pub fn call_count(&self) -> usize {
        self.call_log.lock().expect("call_log mutex poisoned").len()
    }
}

#[async_trait::async_trait]
impl LlmBackend for LenientBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let prompt = req
            .prompt_template
            .expect("LenientBackend services deterministic-spine templated requests");
        let inputs_canonical = serde_json::to_string(&req.inputs).unwrap_or_default();
        self.call_log
            .lock()
            .expect("call_log mutex poisoned")
            .push((prompt, inputs_canonical));
        Ok(match prompt {
            PromptId::Classify => self.classify_response.clone(),
            PromptId::Stage1Surface => self.stage1_surface_response.clone(),
            PromptId::Stage2Edges => json!([]),
            PromptId::Subcarve => json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "policy declined",
            }),
        })
    }

    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call(req)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

fn default_rust_library_classify() -> Value {
    json!({
        "kind": "rust-library",
        "language": "rust",
        "build_system": "cargo",
        "evidence_grade": "medium",
        "evidence_fields": [],
        "rationale": "default lenient",
        "is_boundary": true,
    })
}

fn default_stage1_surface() -> Value {
    json!({"purpose": "stub", "notes": ""})
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_llm::ResponseSchema;

    fn fp() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: "test-backend".into(),
            backend_version: "0".into(),
        }
    }

    fn req(prompt: PromptId) -> LlmRequest {
        LlmRequest::from_template(prompt, json!({}), ResponseSchema::accept_any())
    }

    #[test]
    fn lenient_backend_constructs_and_returns_decline() {
        let backend = LenientBackend::new(fp());

        // Subcarve returns the "decline" shape.
        let subcarve = backend
            .call(&req(PromptId::Subcarve))
            .expect("subcarve must succeed");
        assert_eq!(subcarve["should_subcarve"], json!(false));
        assert_eq!(subcarve["sub_dirs"], json!([]));

        // Classify defaults to rust-library.
        let classify = backend
            .call(&req(PromptId::Classify))
            .expect("classify must succeed");
        assert_eq!(classify["kind"], json!("rust-library"));
        assert_eq!(classify["language"], json!("rust"));

        // Stage1Surface and Stage2Edges return non-empty defaults.
        let surface = backend
            .call(&req(PromptId::Stage1Surface))
            .expect("stage1 surface must succeed");
        assert_eq!(surface["purpose"], json!("stub"));

        let edges = backend
            .call(&req(PromptId::Stage2Edges))
            .expect("stage2 edges must succeed");
        assert_eq!(edges, json!([]));

        // The backend logged every call.
        assert_eq!(backend.call_count(), 4);
        let calls = backend.calls();
        assert_eq!(
            calls,
            vec![
                PromptId::Subcarve,
                PromptId::Classify,
                PromptId::Stage1Surface,
                PromptId::Stage2Edges,
            ]
        );

        // Fingerprint is stable across calls.
        assert_eq!(backend.fingerprint(), fp());
        assert_eq!(backend.fingerprint(), backend.fingerprint());
    }

    #[test]
    fn with_classify_overrides_default_classification() {
        let custom = json!({
            "kind": "python-package",
            "language": "python",
            "evidence_grade": "strong",
            "evidence_fields": [],
            "rationale": "custom",
            "is_boundary": true,
        });
        let backend = LenientBackend::with_classify(fp(), custom.clone());

        let classify = backend
            .call(&req(PromptId::Classify))
            .expect("classify must succeed");
        assert_eq!(classify, custom);
    }

    #[test]
    fn calls_with_inputs_records_canonical_inputs() {
        let backend = LenientBackend::new(fp());
        let payload = json!({"k": "v"});
        let r = LlmRequest::from_template(
            PromptId::Classify,
            payload.clone(),
            ResponseSchema::accept_any(),
        );
        backend.call(&r).expect("call must succeed");

        let logged = backend.calls_with_inputs();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].0, PromptId::Classify);
        assert_eq!(logged[0].1, serde_json::to_string(&payload).unwrap());
    }
}
