//! In-memory deterministic LLM backend for unit tests. Rejects any
//! request not pre-registered with a canned response — so an
//! accidental LLM call from code that is supposed to short-circuit
//! (pins, deterministic classifier rules) fails loud.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

use crate::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};

pub struct TestBackend {
    canned: Mutex<HashMap<(PromptId, String), Value>>,
    fingerprint: LlmFingerprint,
}

impl TestBackend {
    pub fn new() -> Self {
        Self {
            canned: Mutex::new(HashMap::new()),
            fingerprint: LlmFingerprint {
                template_sha: [0u8; 32],
                ontology_sha: [0u8; 32],
                model_id: "test-backend".to_string(),
                backend_version: "0".to_string(),
            },
        }
    }

    pub fn with_fingerprint(fingerprint: LlmFingerprint) -> Self {
        Self {
            canned: Mutex::new(HashMap::new()),
            fingerprint,
        }
    }

    /// Register a canned response for the given `(prompt, inputs)`
    /// pair. If the engine calls the backend with the exact same
    /// prompt id and inputs, the recorded value is returned; any
    /// other call errors.
    pub fn respond(&self, prompt: PromptId, inputs: Value, response: Value) {
        let key = (prompt, canonical_key(&inputs));
        self.canned
            .lock()
            .expect("canned map poisoned")
            .insert(key, response);
    }
}

impl Default for TestBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmBackend for TestBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let key = (req.prompt_template, canonical_key(&req.inputs));
        self.canned
            .lock()
            .expect("canned map poisoned")
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                LlmError::TestBackendMiss(format!(
                    "no canned response for {:?} with inputs {}",
                    req.prompt_template, key.1,
                ))
            })
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

/// Canonical string form of a JSON value used as a map key. Object
/// keys are already sorted by `serde_json::Map` (default build), so
/// `to_string` produces a deterministic representation.
fn canonical_key(value: &Value) -> String {
    serde_json::to_string(value).expect("JSON value must serialise")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResponseSchema;
    use serde_json::json;

    fn req(prompt: PromptId, inputs: Value) -> LlmRequest {
        LlmRequest {
            prompt_template: prompt,
            inputs,
            schema: ResponseSchema::accept_any(),
        }
    }

    #[test]
    fn canned_response_roundtrips() {
        let backend = TestBackend::new();
        backend.respond(
            PromptId::Classify,
            json!({ "dir": "src/lib" }),
            json!({ "kind": "rust-library" }),
        );

        let got = backend
            .call(&req(PromptId::Classify, json!({ "dir": "src/lib" })))
            .expect("should return canned response");

        assert_eq!(got, json!({ "kind": "rust-library" }));
    }

    #[test]
    fn unmapped_input_errors() {
        let backend = TestBackend::new();
        backend.respond(
            PromptId::Classify,
            json!({ "dir": "src/lib" }),
            json!({ "kind": "rust-library" }),
        );

        let err = backend
            .call(&req(PromptId::Classify, json!({ "dir": "src/other" })))
            .unwrap_err();

        assert!(matches!(err, LlmError::TestBackendMiss(_)));
    }

    #[test]
    fn different_prompt_id_does_not_match() {
        let backend = TestBackend::new();
        backend.respond(
            PromptId::Classify,
            json!({ "dir": "src/lib" }),
            json!({ "kind": "rust-library" }),
        );

        // Same inputs, different prompt id — should miss.
        let err = backend
            .call(&req(PromptId::Subcarve, json!({ "dir": "src/lib" })))
            .unwrap_err();

        assert!(matches!(err, LlmError::TestBackendMiss(_)));
    }

    #[test]
    fn canonical_key_is_insensitive_to_object_field_order() {
        // serde_json::Value's Map uses BTreeMap by default (no
        // preserve_order feature), so two `Value`s constructed with
        // fields in different orders produce the same canonical key.
        let a = json!({ "a": 1, "b": 2 });
        let b = json!({ "b": 2, "a": 1 });
        assert_eq!(canonical_key(&a), canonical_key(&b));
    }

    #[test]
    fn fingerprint_defaults_are_test_backend() {
        let fp = TestBackend::new().fingerprint();
        assert_eq!(fp.model_id, "test-backend");
    }

    #[test]
    fn fingerprint_is_configurable() {
        let custom = LlmFingerprint {
            template_sha: [9u8; 32],
            ontology_sha: [7u8; 32],
            model_id: "custom-model".to_string(),
            backend_version: "v42".to_string(),
        };
        let backend = TestBackend::with_fingerprint(custom.clone());

        let got = backend.fingerprint();

        assert_eq!(got.model_id, custom.model_id);
        assert_eq!(got.backend_version, custom.backend_version);
        assert_eq!(got.template_sha, custom.template_sha);
        assert_eq!(got.ontology_sha, custom.ontology_sha);
    }
}
