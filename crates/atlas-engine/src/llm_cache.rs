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
//!
//! ## Two-tier write-through (PR-10)
//!
//! The cache is a write-through wrapper over the persistent
//! content-addressed cache (`crate::cache::PersistentCache`). The
//! in-memory layer dedupes within a process; the persistent layer
//! gives "zero LLM calls on a fresh-process re-run". Lookup is
//! L1 (in-memory) → L2 (persistent) → backend; success at every step
//! seeds the layers above so the next call short-circuits earlier.
//!
//! The two layers use different keys by design — the in-memory key is
//! request-shape-derived and the persistent key is stage-fingerprint-
//! derived (see `crate::cache::FingerprintBuilder` for the contributors
//! per stage). They are not interchangeable. Production callers go
//! through [`LlmResponseCache::call_cached_with_fp`], which knows both
//! keys; the legacy [`LlmResponseCache::call_cached`] keeps the v1
//! signature for tests that don't care about the persistent layer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use atlas_index::Stage;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::Value;

use crate::cache::{PersistentCache, Sha256Hex};

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
///
/// May optionally wrap a [`PersistentCache`] (PR-10): when present,
/// [`LlmResponseCache::call_cached_with_fp`] consults it on in-memory
/// miss before invoking the backend, and writes any backend response
/// back through it. Both layers are seeded on a hit so the next call
/// short-circuits earlier.
#[derive(Default, Clone)]
pub struct LlmResponseCache {
    inner: Arc<Mutex<Inner>>,
    /// Optional persistent backing store. `None` for tests that only
    /// exercise the in-memory layer; the production pipeline opens a
    /// `PersistentCache` rooted at `<output>/.atlas/cache/` and
    /// installs it via [`LlmResponseCache::new_with_persistent`].
    persistent: Option<PersistentCache>,
}

type PersistHook = Arc<dyn Fn(&LlmResponseCache) + Send + Sync>;

#[derive(Default)]
struct Inner {
    entries: HashMap<LlmCacheKey, Arc<Value>>,
    call_count: u64,
    error_count: u64,
    persist_hook: Option<PersistHook>,
    /// Number of persistent-cache hits served without invoking the
    /// backend. Distinct from the in-memory hit path (which simply
    /// does not increment any counter). Tests use this to assert the
    /// "fresh process re-run hits the persistent cache" contract.
    persistent_hits: u64,
}

impl LlmResponseCache {
    pub fn new() -> Self {
        LlmResponseCache::default()
    }

    /// Construct a cache backed by `persistent`. Every successful
    /// backend call writes through to both layers; every in-memory miss
    /// consults the persistent layer before falling through to the
    /// backend.
    pub fn new_with_persistent(persistent: PersistentCache) -> Self {
        LlmResponseCache {
            inner: Arc::default(),
            persistent: Some(persistent),
        }
    }

    /// Persistent layer accessor (read-only). Returns `None` when the
    /// cache was constructed without a persistent backing store, e.g.
    /// in tests that only exercise the in-memory dedup.
    pub fn persistent(&self) -> Option<&PersistentCache> {
        self.persistent.as_ref()
    }

    /// Number of persistent-cache hits served without invoking the
    /// backend, since cache construction or the most recent
    /// [`LlmResponseCache::clear`] call. Tests use this to assert the
    /// "fresh-process re-run hits the persistent cache" contract.
    pub fn persistent_hit_count(&self) -> u64 {
        self.inner
            .lock()
            .expect("llm cache poisoned")
            .persistent_hits
    }

    /// Backend-call count recorded since cache construction or the
    /// most recent [`LlmResponseCache::clear`] call. Tests use this to
    /// assert cache-hit behaviour.
    pub fn call_count(&self) -> u64 {
        self.inner.lock().expect("llm cache poisoned").call_count
    }

    /// Number of cache misses where the wrapped backend returned an
    /// error. Increments only on an error path — successful misses go to
    /// [`LlmResponseCache::call_count`] instead. Lets pipeline summaries
    /// distinguish "no calls were needed" (call_count=0, error_count=0)
    /// from "every call attempted, every call failed" (call_count=0,
    /// error_count>0).
    pub fn error_count(&self) -> u64 {
        self.inner.lock().expect("llm cache poisoned").error_count
    }

    /// Lookup-or-populate (in-memory only). Returns the cached
    /// response if present; otherwise calls `backend.call(request)`,
    /// stores the result, and returns it.
    ///
    /// Does **not** consult the persistent layer. Production call sites
    /// (L3/L5/L6) go through [`LlmResponseCache::call_cached_with_fp`],
    /// which threads the stage / fingerprint required to consult the
    /// content-addressed store. This signature is preserved as a
    /// back-compat entry point for tests that only care about
    /// per-process dedup.
    pub fn call_cached(
        &self,
        backend: &dyn LlmBackend,
        request: &LlmRequest,
    ) -> Result<Arc<Value>, LlmError> {
        let fingerprint = backend.fingerprint();
        let key = LlmCacheKey::from_request(&fingerprint, request);
        if let Some(value) = self.in_memory_get(&key) {
            return Ok(value);
        }
        self.call_through_backend_and_seed(backend, request, &key, None)
    }

    /// Two-tier lookup-or-populate. Order:
    ///
    /// 1. In-memory hit: short-circuits without consulting the
    ///    persistent layer.
    /// 2. Persistent hit (when a [`PersistentCache`] is attached):
    ///    decodes the blob, seeds the in-memory cache, increments
    ///    [`LlmResponseCache::persistent_hit_count`], returns.
    /// 3. Backend call: result is encoded and written to the
    ///    persistent layer (best-effort; persistence failures are
    ///    warned but do not fail the call), then stored in-memory.
    ///
    /// `stage` and `fingerprint` come from the L-stage caller and
    /// describe the persistent-cache key shape required by design
    /// §5.4 / §8.1. The in-memory key is request-shape-derived
    /// independently — see [`LlmCacheKey::from_request`] — so the two
    /// layers stay in sync only because the caller passes the same
    /// request to both lookups.
    pub fn call_cached_with_fp(
        &self,
        stage: Stage,
        fingerprint: &Sha256Hex,
        backend: &dyn LlmBackend,
        request: &LlmRequest,
    ) -> Result<Arc<Value>, LlmError> {
        let backend_fp = backend.fingerprint();
        let key = LlmCacheKey::from_request(&backend_fp, request);

        // Tier 1: in-memory.
        if let Some(value) = self.in_memory_get(&key) {
            return Ok(value);
        }

        // Tier 2: persistent. A read failure (e.g. corrupt blob, EIO)
        // degrades to a backend miss with a warning; the run still
        // completes and the next write replaces the bad blob.
        if let Some(persistent) = self.persistent.as_ref() {
            match persistent.get(stage, fingerprint) {
                Ok(Some(blob)) => match decode_blob(&blob) {
                    Ok(value) => {
                        let value = Arc::new(value);
                        let mut inner = self.inner.lock().expect("llm cache poisoned");
                        inner.entries.insert(key, value.clone());
                        inner.persistent_hits += 1;
                        return Ok(value);
                    }
                    Err(err) => {
                        eprintln!(
                            "warning: persistent cache blob for ({stage:?}, {fingerprint}) \
                             failed to decode ({err}); falling through to backend call"
                        );
                    }
                },
                Ok(None) => {}
                Err(err) => {
                    eprintln!(
                        "warning: persistent cache read for ({stage:?}, {fingerprint}) failed \
                         ({err}); falling through to backend call"
                    );
                }
            }
        }

        // Tier 3: backend.
        self.call_through_backend_and_seed(backend, request, &key, Some((stage, fingerprint)))
    }

    /// Low-level helper used by both [`LlmResponseCache::call_cached`]
    /// and [`LlmResponseCache::call_cached_with_fp`]. Calls the
    /// backend, seeds the in-memory cache, and (when a persistent key
    /// is supplied) encodes the response into the persistent layer.
    fn call_through_backend_and_seed(
        &self,
        backend: &dyn LlmBackend,
        request: &LlmRequest,
        key: &LlmCacheKey,
        persistent_key: Option<(Stage, &Sha256Hex)>,
    ) -> Result<Arc<Value>, LlmError> {
        // Invoke the backend without holding the cache lock; a
        // concurrent call for the same key may double-fetch, but
        // backend responses are equal by the `LlmBackend` invariants
        // so the worst case is one redundant call, not a correctness
        // problem.
        let value = match backend.call(request) {
            Ok(v) => v,
            Err(e) => {
                self.inner.lock().expect("llm cache poisoned").error_count += 1;
                return Err(e);
            }
        };
        let value = Arc::new(value);

        // Persistent write happens before in-memory seeding so a
        // crash between the two leaves the persistent layer hot
        // (no spurious miss next run). A failed persistent write is
        // non-fatal: the run still gets the in-memory cache, and
        // the next save attempt will retry implicitly.
        if let Some((stage, fingerprint)) = persistent_key {
            if let Some(persistent) = self.persistent.as_ref() {
                match encode_blob(value.as_ref()) {
                    Ok(blob) => {
                        if let Err(err) = persistent.put(stage, fingerprint, &blob) {
                            eprintln!(
                                "warning: persistent cache write for ({stage:?}, {fingerprint}) \
                                 failed ({err:#}); the in-memory layer still has the entry"
                            );
                        }
                    }
                    Err(err) => {
                        eprintln!(
                            "warning: persistent cache encode for ({stage:?}, {fingerprint}) \
                             failed ({err}); the in-memory layer still has the entry"
                        );
                    }
                }
            }
        }

        let hook = {
            let mut inner = self.inner.lock().expect("llm cache poisoned");
            inner.call_count += 1;
            inner.entries.insert(key.clone(), value.clone());
            inner.persist_hook.clone()
        };
        if let Some(hook) = hook {
            hook(self);
        }
        Ok(value)
    }

    fn in_memory_get(&self, key: &LlmCacheKey) -> Option<Arc<Value>> {
        self.inner
            .lock()
            .expect("llm cache poisoned")
            .entries
            .get(key)
            .cloned()
    }

    /// Register a callback invoked after every successful cache insert.
    /// Drivers (atlas-cli) use this to persist the cache as work proceeds —
    /// without it, an aborted run loses every response since the start.
    pub fn set_persist_hook<F>(&self, hook: F)
    where
        F: Fn(&LlmResponseCache) + Send + Sync + 'static,
    {
        self.inner.lock().expect("llm cache poisoned").persist_hook = Some(Arc::new(hook));
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("llm cache poisoned");
        inner.entries.clear();
        inner.call_count = 0;
        inner.error_count = 0;
        inner.persistent_hits = 0;
    }

    /// Snapshot of every `(key, response)` pair currently cached.
    /// Exposed so drivers (atlas-cli) can persist the cache across
    /// process invocations — the "zero LLM calls on no-op re-run"
    /// contract depends on reloading the prior run's cache into a
    /// fresh database.
    pub fn entries_snapshot(&self) -> Vec<(LlmCacheKey, Arc<Value>)> {
        let inner = self.inner.lock().expect("llm cache poisoned");
        inner
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Install a pre-computed entry in the cache without invoking the
    /// backend. Used by the CLI to seed the cache from a prior run's
    /// on-disk snapshot.
    pub fn seed(&self, key: LlmCacheKey, value: Arc<Value>) {
        let mut inner = self.inner.lock().expect("llm cache poisoned");
        inner.entries.insert(key, value);
    }
}

/// Encode an LLM response into the byte form stored in the persistent
/// cache. Phase 1 ships uncompressed JSON; the cache key explicitly
/// does not include the compression algorithm (per design §11.2.7) so
/// a future PR can swap the codec without invalidating prior entries
/// — but we still hide the format behind a single helper so any such
/// swap is one edit, not many.
fn encode_blob(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(value)
}

/// Inverse of [`encode_blob`]. Returns the raw `Value`; callers wrap
/// it in `Arc` themselves.
fn decode_blob(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(bytes)
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
        let a =
            LlmCacheKey::from_request(&f, &req(PromptId::Stage1Surface, json!({ "a": 1, "b": 2 })));
        let b =
            LlmCacheKey::from_request(&f, &req(PromptId::Stage1Surface, json!({ "b": 2, "a": 1 })));
        assert_eq!(a, b);
    }

    #[test]
    fn key_differs_when_prompt_id_differs() {
        let f = fp("m");
        let a = LlmCacheKey::from_request(&f, &req(PromptId::Stage1Surface, json!({ "id": "A" })));
        let b = LlmCacheKey::from_request(&f, &req(PromptId::Stage2Edges, json!({ "id": "A" })));
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
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap();
        assert_eq!(cache.call_count(), 1);

        let second = cache
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap();

        assert_eq!(*first, *second);
        assert_eq!(
            cache.call_count(),
            1,
            "second identical call must hit cache"
        );
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
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap();
        cache
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "B" })),
            )
            .unwrap();
        assert_eq!(cache.call_count(), 2);
    }

    #[test]
    fn error_count_increments_on_backend_error_and_call_count_does_not() {
        // TestBackend with no canned responses errors with TestBackendMiss
        // on every request — exercises the miss-then-error path.
        let backend = TestBackend::with_fingerprint(fp("m"));
        let cache = LlmResponseCache::new();

        let err = cache
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap_err();
        assert!(matches!(err, LlmError::TestBackendMiss(_)), "{err:?}");
        assert_eq!(cache.call_count(), 0);
        assert_eq!(cache.error_count(), 1);

        // A second erroring call increments error_count again.
        let _ = cache
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "B" })),
            )
            .unwrap_err();
        assert_eq!(cache.call_count(), 0);
        assert_eq!(cache.error_count(), 2);
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
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap();
        assert_eq!(cache.call_count(), 1);

        cache.clear();
        assert_eq!(cache.call_count(), 0);

        cache
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap();
        assert_eq!(cache.call_count(), 1);
    }

    // ----- PR-10: persistent-layer behaviour -----------------------

    fn temp_persistent() -> (PersistentCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = PersistentCache::open(dir.path()).expect("open");
        (cache, dir)
    }

    #[test]
    fn persistent_layer_serves_a_hit_without_backend_call() {
        // First, populate the persistent layer via one cache holding
        // a backend; then drop the in-memory state by constructing a
        // fresh `LlmResponseCache` over the *same* on-disk directory
        // and prove the second call does not invoke the backend.
        let (persistent, _dir) = temp_persistent();
        let backend = TestBackend::with_fingerprint(fp("m"));
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "A" }),
            json!({ "purpose": "p" }),
        );
        let fingerprint = "deadbeef".to_string();

        // Cold: backend is invoked, persistent layer is populated.
        let cold = LlmResponseCache::new_with_persistent(persistent.clone());
        let v1 = cold
            .call_cached_with_fp(
                Stage::L3,
                &fingerprint,
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .expect("cold call");
        assert_eq!(cold.call_count(), 1);
        assert_eq!(cold.persistent_hit_count(), 0);

        // Warm: a fresh in-memory layer over the same persistent
        // store sees the hit and never touches the backend.
        let warm = LlmResponseCache::new_with_persistent(persistent);
        let v2 = warm
            .call_cached_with_fp(
                Stage::L3,
                &fingerprint,
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .expect("warm call");
        assert_eq!(warm.call_count(), 0, "persistent hit must not call backend");
        assert_eq!(warm.persistent_hit_count(), 1);
        assert_eq!(*v1, *v2);
    }

    #[test]
    fn distinct_fingerprints_miss_persistent_layer_independently() {
        let (persistent, _dir) = temp_persistent();
        let backend = TestBackend::with_fingerprint(fp("m"));
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "A" }),
            json!({ "purpose": "A" }),
        );
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "B" }),
            json!({ "purpose": "B" }),
        );
        let cache = LlmResponseCache::new_with_persistent(persistent);

        cache
            .call_cached_with_fp(
                Stage::L3,
                &"fp-a".to_string(),
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap();
        cache
            .call_cached_with_fp(
                Stage::L3,
                &"fp-b".to_string(),
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "B" })),
            )
            .unwrap();
        assert_eq!(cache.call_count(), 2);
    }

    #[test]
    fn in_memory_hit_short_circuits_persistent_lookup() {
        // After the first call seeds the in-memory layer, the second
        // call must not even consult the persistent store.
        // Indirect proof: persistent_hit_count stays at 0 across
        // repeated calls in the same cache instance.
        let (persistent, _dir) = temp_persistent();
        let backend = TestBackend::with_fingerprint(fp("m"));
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "A" }),
            json!({ "purpose": "p" }),
        );
        let cache = LlmResponseCache::new_with_persistent(persistent);
        let request = req(PromptId::Stage1Surface, json!({ "id": "A" }));

        cache
            .call_cached_with_fp(Stage::L3, &"abc".to_string(), &backend, &request)
            .unwrap();
        assert_eq!(cache.call_count(), 1);
        assert_eq!(cache.persistent_hit_count(), 0);

        cache
            .call_cached_with_fp(Stage::L3, &"abc".to_string(), &backend, &request)
            .unwrap();
        assert_eq!(cache.call_count(), 1, "second call must hit in-memory");
        assert_eq!(
            cache.persistent_hit_count(),
            0,
            "in-memory hit must short-circuit persistent layer"
        );
    }

    #[test]
    fn call_cached_without_persistent_does_not_touch_disk() {
        // The legacy entry point is preserved for tests; constructing
        // a cache without `new_with_persistent` must not require a
        // disk path.
        let backend = TestBackend::with_fingerprint(fp("m"));
        backend.respond(
            PromptId::Stage1Surface,
            json!({ "id": "A" }),
            json!({ "purpose": "p" }),
        );
        let cache = LlmResponseCache::new();
        assert!(cache.persistent().is_none());
        cache
            .call_cached(
                &backend,
                &req(PromptId::Stage1Surface, json!({ "id": "A" })),
            )
            .unwrap();
        assert_eq!(cache.call_count(), 1);
        assert_eq!(cache.persistent_hit_count(), 0);
    }

    #[test]
    fn encode_decode_round_trip_preserves_value() {
        let v = json!({ "purpose": "round-trip", "n": 42 });
        let blob = encode_blob(&v).unwrap();
        let back = decode_blob(&blob).unwrap();
        assert_eq!(v, back);
    }
}
