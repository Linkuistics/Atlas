//! In-process LLM response cache, keyed by
//! `(LlmFingerprint, PromptId, canonical-JSON(inputs))`.
//!
//! Lives alongside [`crate::db::AtlasDatabase`] because the backend
//! itself is a non-Salsa field on the database — Salsa 0.26 does not
//! expose a downcast from `&dyn salsa::Database` to `&AtlasDatabase`,
//! so LLM-call memoisation cannot be a `#[salsa::tracked]` query. See
//! the "LLM memoisation preferred strategy" memory for rationale.
//!
//! The cache is sound in the Atlas sense: every input that affects the
//! response shows up either in the fingerprint (model / template / ont)
//! or in the request inputs (component id, tree shas, peer surfaces).
//! Two lookups with equal keys MUST produce equal responses for the
//! memoisation contract to hold — backends guarantee this per the
//! `LlmBackend` invariants.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::Value;

/// Canonical cache key. Fingerprint goes in first so responses stay
/// valid across `set_llm_fingerprint` churn even when the prompt inputs
/// did not move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmCacheKey {
    pub fingerprint: LlmFingerprint,
    pub prompt: PromptId,
    /// Canonical JSON of `LlmRequest.inputs`. `serde_json::Value`'s
    /// default object representation is `BTreeMap`, so `to_string`
    /// serialises keys in sorted order.
    pub inputs: String,
}

impl LlmCacheKey {
    pub fn from_request(fingerprint: &LlmFingerprint, request: &LlmRequest) -> Self {
        let inputs = serde_json::to_string(&request.inputs)
            .expect("LlmRequest.inputs is a JSON Value and must serialise");
        LlmCacheKey {
            fingerprint: fingerprint.clone(),
            prompt: request.prompt_template,
            inputs,
        }
    }
}

impl std::hash::Hash for LlmCacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.fingerprint.template_sha.hash(state);
        self.fingerprint.ontology_sha.hash(state);
        self.fingerprint.model_id.hash(state);
        self.fingerprint.backend_version.hash(state);
        self.prompt.hash(state);
        self.inputs.hash(state);
    }
}

/// Backend-call cache shared across the whole engine run. Holds the
/// response `Value` in `Arc` so the `.call_cached()` wrapper can hand
/// out cheap clones. The miss-count field feeds the cache-behaviour
/// tests.
#[derive(Default, Clone)]
pub struct LlmResponseCache {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<LlmCacheKey, Arc<Value>>,
    call_count: u64,
}

impl LlmResponseCache {
    pub fn new() -> Self {
        LlmResponseCache::default()
    }

    /// Backend-call count recorded since cache construction or the
    /// most recent [`LlmResponseCache::clear`] call. Tests use this to
    /// assert cache-hit behaviour.
    pub fn call_count(&self) -> u64 {
        self.inner.lock().expect("llm cache poisoned").call_count
    }

    /// Lookup-or-populate. Returns the cached response if present;
    /// otherwise calls `backend.call(request)`, stores the result, and
    /// returns it.
    pub fn call_cached(
        &self,
        backend: &dyn LlmBackend,
        request: &LlmRequest,
    ) -> Result<Arc<Value>, LlmError> {
        let fingerprint = backend.fingerprint();
        let key = LlmCacheKey::from_request(&fingerprint, request);
        if let Some(value) = self
            .inner
            .lock()
            .expect("llm cache poisoned")
            .entries
            .get(&key)
            .cloned()
        {
            return Ok(value);
        }

        // Miss: invoke backend without holding the lock; store on the
        // way out. A concurrent call for the same key may double-fetch,
        // but the responses are equal by the backend invariant so the
        // worst case is one redundant call, not a correctness problem.
        let value = backend.call(request)?;
        let value = Arc::new(value);
        let mut inner = self.inner.lock().expect("llm cache poisoned");
        inner.call_count += 1;
        inner.entries.insert(key, value.clone());
        Ok(value)
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("llm cache poisoned");
        inner.entries.clear();
        inner.call_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_llm::{LlmFingerprint, PromptId, ResponseSchema, TestBackend};
    use serde_json::json;

    fn fp(model: &str) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [1u8; 32],
            ontology_sha: [2u8; 32],
            model_id: model.to_string(),
            backend_version: "v0".to_string(),
        }
    }

    fn req(prompt: PromptId, inputs: serde_json::Value) -> LlmRequest {
        LlmRequest {
            prompt_template: prompt,
            inputs,
            schema: ResponseSchema::accept_any(),
        }
    }

    #[test]
    fn key_is_stable_across_equal_inputs_regardless_of_field_order() {
        let f = fp("m");
        let a = LlmCacheKey::from_request(
            &f,
            &req(PromptId::Stage1Surface, json!({ "a": 1, "b": 2 })),
        );
        let b = LlmCacheKey::from_request(
            &f,
            &req(PromptId::Stage1Surface, json!({ "b": 2, "a": 1 })),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn key_differs_when_prompt_id_differs() {
        let f = fp("m");
        let a = LlmCacheKey::from_request(
            &f,
            &req(PromptId::Stage1Surface, json!({ "id": "A" })),
        );
        let b = LlmCacheKey::from_request(
            &f,
            &req(PromptId::Stage2Edges, json!({ "id": "A" })),
        );
        assert_ne!(a, b);
    }

    #[test]
    fn key_differs_when_fingerprint_model_differs() {
        let a = LlmCacheKey::from_request(
            &fp("m1"),
            &req(PromptId::Stage1Surface, json!({ "id": "A" })),
        );
        let b = LlmCacheKey::from_request(
            &fp("m2"),
            &req(PromptId::Stage1Surface, json!({ "id": "A" })),
        );
        assert_ne!(a, b);
    }

    #[test]
    fn second_call_with_equal_inputs_is_a_cache_hit() {
        let backend = TestBackend::with_fingerprint(fp("m"));
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "A" }),
            json!({ "purpose": "p" }),
        );
        let cache = LlmResponseCache::new();

        let first = cache
            .call_cached(&backend, &req(PromptId::Stage1Surface, json!({ "id": "A" })))
            .unwrap();
        assert_eq!(cache.call_count(), 1);

        let second = cache
            .call_cached(&backend, &req(PromptId::Stage1Surface, json!({ "id": "A" })))
            .unwrap();

        assert_eq!(*first, *second);
        assert_eq!(cache.call_count(), 1, "second identical call must hit cache");
    }

    #[test]
    fn differing_inputs_cause_a_second_backend_call() {
        let backend = TestBackend::with_fingerprint(fp("m"));
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "A" }),
            json!({ "purpose": "A-purpose" }),
        );
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "B" }),
            json!({ "purpose": "B-purpose" }),
        );
        let cache = LlmResponseCache::new();

        cache
            .call_cached(&backend, &req(PromptId::Stage1Surface, json!({ "id": "A" })))
            .unwrap();
        cache
            .call_cached(&backend, &req(PromptId::Stage1Surface, json!({ "id": "B" })))
            .unwrap();
        assert_eq!(cache.call_count(), 2);
    }

    #[test]
    fn clear_resets_entries_and_counter() {
        let backend = TestBackend::with_fingerprint(fp("m"));
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "A" }),
            json!({ "purpose": "p" }),
        );
        let cache = LlmResponseCache::new();
        cache
            .call_cached(&backend, &req(PromptId::Stage1Surface, json!({ "id": "A" })))
            .unwrap();
        assert_eq!(cache.call_count(), 1);

        cache.clear();
        assert_eq!(cache.call_count(), 0);

        cache
            .call_cached(&backend, &req(PromptId::Stage1Surface, json!({ "id": "A" })))
            .unwrap();
        assert_eq!(cache.call_count(), 1);
    }
}
