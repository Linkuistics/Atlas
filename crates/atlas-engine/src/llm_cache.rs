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
use std::path::Path;
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

// ============================================================================
// Phase 7 PR-2: multi-shot transcript cache (call_agent_cached)
// ============================================================================
//
// Multi-shot transcript-cache extension of `call_cached_with_fp`
// (recast §6.1). Single-shot calls (today's L3/L5/L6 backbone) keep
// using `call_cached_with_fp`; multi-shot agent runs (Phase 7+) use
// `call_agent_cached`. Both share the in-memory `LlmResponseCache`
// instance but use disjoint persistent-layer paths (`cache/<stage>/...`
// vs. `cache/agents/<stage>/...`).

use sha2::{Digest, Sha256};

use crate::atomic_write::atomic_write_pair;
use crate::cache::layout::{agents_output_path, agents_transcript_path};

/// Confidence grade attached to an agent result. Lifted from the
/// `atlas_agents::Grade` shape (kept here as a separate enum to
/// preserve the `atlas-engine` → `atlas-agents` layering — engine is
/// lower-level, agents depends on engine, not the other way around).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentGrade {
    Strong,
    Moderate,
    Weak,
    Declines,
}

/// Cache-key contributors for the transcript cache (recast §6.1).
///
/// Every field that can affect the agent's output goes into the key,
/// so cache hits are sound. `transport_flavour` is stored as a string
/// (the snake_case wire form from `atlas_agents::TransportFlavour::as_str`)
/// to avoid an engine → agents dependency cycle.
#[derive(Debug, Clone)]
pub struct AgentInputFingerprint {
    pub stage_id: String,
    pub agent_id: String,
    pub agent_version: String,
    pub prompt_template_sha: [u8; 32],
    pub tool_catalog_sha: [u8; 32],
    pub model_id: String,
    pub backend_version: String,
    /// Wire form of `atlas_agents::TransportFlavour::as_str` —
    /// e.g. "claude_code", "codex". Kept as `String` to keep
    /// engine layered below atlas-agents.
    pub transport_flavour: String,
    pub target_input_shas: Vec<[u8; 32]>,
    pub iteration_number: u32,
    pub prior_model_sha: Option<[u8; 32]>,
}

impl AgentInputFingerprint {
    /// Render the fingerprint as a 64-character hex SHA-256 used as the
    /// `<sha>` file-stem in the cache layout. Every field is folded in,
    /// in declaration order, with `\x00` separators so distinct fields
    /// cannot collide via string concatenation.
    pub fn to_cache_key(&self) -> Sha256Hex {
        let mut hasher = Sha256::new();
        hasher.update(self.stage_id.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.agent_id.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.agent_version.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.prompt_template_sha);
        hasher.update(b"\x00");
        hasher.update(self.tool_catalog_sha);
        hasher.update(b"\x00");
        hasher.update(self.model_id.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.backend_version.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.transport_flavour.as_bytes());
        hasher.update(b"\x00");
        for sha in &self.target_input_shas {
            hasher.update(sha);
            hasher.update(b"\x00");
        }
        hasher.update(self.iteration_number.to_le_bytes());
        hasher.update(b"\x00");
        if let Some(prior) = self.prior_model_sha {
            hasher.update(b"1");
            hasher.update(prior);
        } else {
            hasher.update(b"0");
        }
        hex::encode(hasher.finalize())
    }
}

/// Spot-check entry: one recorded `(path_sha)` pair that the agent
/// fingerprinted as one of its `target_input_shas`. Used by
/// `call_agent_cached` to evict an entry whose recorded file_sha no
/// longer matches the current value (recast §6.3).
///
/// The path identifies the input file (for forensic logging); the
/// `recorded_sha` is the file's sha at the time the cache was written.
/// On read we re-hash the current file via `current_sha_fn` and evict
/// if they differ.
#[derive(Debug, Clone)]
pub struct FingerprintInputSpotCheck {
    pub path: String,
    pub recorded_sha: [u8; 32],
}

/// Minimal placeholder shape for the multi-shot agent request. The
/// production runtime (PR-4+) extends this with the actual
/// `request_body` / `tools` / `audit_policy` / `max_steps` fields; for
/// PR-2 the cache only needs *some* payload to thread through the
/// compute closure, and the fingerprint carries the cache-key inputs
/// directly.
///
/// `#[non_exhaustive]` locks in additive-compatibility for PR-4+: new
/// fields can be added without breaking downstream construction sites,
/// provided every construction site goes through [`AgentRequest::new`]
/// rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentRequest {
    /// Opaque payload (e.g. serialised request body). Not hashed into
    /// the cache key — the fingerprint already carries every cache-key
    /// contributor; the request body itself is rendered from those
    /// contributors and is therefore redundant. Carried here only so
    /// the `compute` closure has access to the payload it needs to
    /// drive the backend.
    pub payload: Vec<u8>,
}

impl AgentRequest {
    /// Construct an `AgentRequest` with `payload`. The only construction
    /// path; struct literals are forbidden by `#[non_exhaustive]` for
    /// out-of-crate callers, and by convention within the crate too.
    pub fn new(payload: Vec<u8>) -> Self {
        Self { payload }
    }
}

/// Minimal placeholder shape for the multi-shot agent result. The
/// production runtime (PR-4+) extends this with the actual evidence /
/// transcript-handle fields; for PR-2 the cache only needs to
/// round-trip `transcript_bytes` + `output_bytes` (the two artefacts
/// the atomic-pair write commits) plus the confidence grade so the
/// audit-fire decision (PR-4+) can read it back.
///
/// `#[non_exhaustive]` locks in additive-compatibility for PR-4+: new
/// fields can be added without breaking downstream construction sites,
/// provided every construction site goes through [`AgentResult::new`]
/// rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentResult {
    pub transcript_bytes: Vec<u8>,
    pub output_bytes: Vec<u8>,
    pub confidence_grade: AgentGrade,
}

impl AgentResult {
    /// Construct an `AgentResult`. The only construction path; struct
    /// literals are forbidden by `#[non_exhaustive]` for out-of-crate
    /// callers, and by convention within the crate too.
    pub fn new(
        transcript_bytes: Vec<u8>,
        output_bytes: Vec<u8>,
        confidence_grade: AgentGrade,
    ) -> Self {
        Self {
            transcript_bytes,
            output_bytes,
            confidence_grade,
        }
    }
}

/// Error type for the multi-shot agent path. Wraps any backend error
/// the `compute` closure surfaces. The cache itself never produces an
/// `AgentError` — all variants come from below.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("backend call failed: {0}")]
    BackendFailed(String),
}

/// Stage identifier for the transcript cache. Re-uses the existing
/// `atlas_index::Stage` enum because the persistent layout shares the
/// `<stage>` directory level with the single-shot cache.
pub type AgentCacheStage = Stage;

impl LlmResponseCache {
    /// Multi-shot transcript-cache lookup-or-populate (recast §6.1).
    ///
    /// Key shape (per [`AgentInputFingerprint::to_cache_key`]):
    /// `sha256(stage_id || agent_id || agent_version || prompt_template_sha
    /// || tool_catalog_sha || model_id || backend_version || transport_flavour
    /// || target_input_shas || iteration_number || prior_model_sha)`.
    ///
    /// Persistent layout:
    /// `<root>/cache/agents/<stage>/<sha>.transcript` +
    /// `<root>/cache/agents/<stage>/<sha>.output`. Atomic-pair writes
    /// via [`crate::atomic_write::atomic_write_pair`]. The two files
    /// either both land or (in the residual half-pair window) the
    /// recorded-fingerprint spot-check evicts on next read.
    ///
    /// `recorded_inputs` is the list of `(path, recorded_sha)` pairs
    /// the agent fingerprinted; on cache hit we re-hash each via
    /// `current_sha_fn` and evict if any differs from `recorded_sha`
    /// (recast §6.3). PR-2 ships the eviction path; PR-4+ wires up the
    /// concrete `current_sha_fn` against the live `AtlasDatabase`.
    ///
    /// On hard fail (the `compute` closure returns `Err`), no cache
    /// write happens (recast §6.4) — the writer is gated on the
    /// `Ok(_)` arm.
    pub fn call_agent_cached(
        &self,
        stage: AgentCacheStage,
        fingerprint: &AgentInputFingerprint,
        recorded_inputs: &[FingerprintInputSpotCheck],
        current_sha_fn: impl Fn(&str) -> Option<[u8; 32]>,
        request: AgentRequest,
        compute: impl FnOnce(&AgentRequest) -> Result<AgentResult, AgentError>,
    ) -> Result<AgentResult, AgentError> {
        let key = fingerprint.to_cache_key();

        // L2 persistent lookup. The in-memory layer is intentionally
        // not consulted here: the multi-shot cache holds raw bytes
        // (transcript + output), not `serde_json::Value`s, so the
        // single-shot in-memory map's value type doesn't fit. A future
        // PR can introduce a parallel multi-shot in-memory layer if
        // profiling proves the L2-only path is too slow; for now the
        // L2 alone is the de-dup boundary.
        if let Some(persistent) = self.persistent.as_ref() {
            if let Some(hit) = read_agent_pair(persistent.root(), stage, &key) {
                // Spot-check recorded fingerprint inputs against the
                // current file_sha view (recast §6.3). On mismatch,
                // evict the half-pair and fall through to recompute.
                let mut stale = false;
                for entry in recorded_inputs {
                    match current_sha_fn(&entry.path) {
                        Some(current) if current == entry.recorded_sha => {}
                        _ => {
                            stale = true;
                            break;
                        }
                    }
                }
                if !stale {
                    return Ok(hit);
                }
                evict_agent_pair(persistent.root(), stage, &key);
            }
        }

        // Backend / compute path.
        let result = compute(&request)?;

        // Write the pair atomically on success only (recast §6.4).
        if let Some(persistent) = self.persistent.as_ref() {
            let transcript = agents_transcript_path(persistent.root(), stage, &key);
            let output = agents_output_path(persistent.root(), stage, &key);
            if let Err(err) = atomic_write_pair(
                &transcript,
                &result.transcript_bytes,
                &output,
                &result.output_bytes,
            ) {
                eprintln!(
                    "warning: transcript cache write for ({stage:?}, {key}) failed ({err}); \
                     the run still completes but the next run will re-compute"
                );
            }
        }

        Ok(result)
    }
}

/// Try to read the `(transcript, output)` pair for `(stage, key)`.
/// Returns `Some(AgentResult)` only when both files exist and the
/// transcript decodes its `confidence_grade` header. Half-pairs
/// (one file present, the other missing — the residual atomic-pair
/// window) are treated as a miss so the recompute path takes over.
///
/// The asymmetric-present case (exactly one of the two files exists)
/// is rare — it is the post-rename-a / pre-rename-b crash residue from
/// `atomic_write_pair` — but when it does occur, downstream debugging
/// needs the signal, so we emit a `tracing::warn!` on detection. The
/// extra two `path.exists()` syscalls on the cold path are negligible
/// next to the diagnostic value.
fn read_agent_pair(root: &Path, stage: Stage, key: &Sha256Hex) -> Option<AgentResult> {
    let transcript_path = agents_transcript_path(root, stage, key);
    let output_path = agents_output_path(root, stage, key);
    let transcript_exists = transcript_path.exists();
    let output_exists = output_path.exists();
    match (transcript_exists, output_exists) {
        (true, true) => {
            let transcript_bytes = std::fs::read(&transcript_path).ok()?;
            let output_bytes = std::fs::read(&output_path).ok()?;

            // The confidence grade is stored as a one-line header at the
            // top of the transcript blob: `# grade: <variant>\n<rest...>`.
            // PR-2 ships the simplest possible framing; PR-4 can swap to
            // a richer wire format (e.g. JSON envelope) without
            // invalidating prior entries by gating the framing version on
            // `backend_version`, which is already in the cache key.
            let (grade, body) = parse_transcript_grade(&transcript_bytes)?;
            Some(AgentResult::new(body, output_bytes, grade))
        }
        (false, false) => None,
        // Half-pair: post-rename-a / pre-rename-b crash residue. Emit a
        // diagnostic so callers can chase the missing rename, then
        // return `None` so the recompute path overwrites the residue.
        (_, _) => {
            tracing::warn!(
                stage = ?stage,
                key = %key,
                transcript_present = transcript_exists,
                output_present = output_exists,
                "half-pair transcript-cache entry detected; treating as miss and recomputing"
            );
            None
        }
    }
}

/// Best-effort half-pair eviction. Removes both files; ignores
/// `NotFound`. Other I/O errors are logged but do not propagate —
/// the recompute path will overwrite either file on success.
fn evict_agent_pair(root: &Path, stage: Stage, key: &Sha256Hex) {
    for path in [
        agents_transcript_path(root, stage, key),
        agents_output_path(root, stage, key),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!(
                "warning: transcript cache eviction of {} failed ({e})",
                path.display()
            ),
        }
    }
}

/// Wire prefix for the transcript-grade framing. The frame is
/// `<PREFIX><label>\n<body>`; encode and decode both go through this
/// constant + the [`grade_label`] / [`grade_from_label`] helpers so the
/// label list lives in exactly one place. PR-4+ subscribers that need
/// to validate the framing import this constant + the public
/// [`parse_transcript_grade`] / [`frame_transcript_with_grade`] pair.
pub const TRANSCRIPT_FRAME_PREFIX: &[u8] = b"# grade: ";

/// Wire label for `grade`. Inverse of [`grade_from_label`]. Single
/// source of truth for the on-disk label set.
fn grade_label(grade: &AgentGrade) -> &'static str {
    match grade {
        AgentGrade::Strong => "strong",
        AgentGrade::Moderate => "moderate",
        AgentGrade::Weak => "weak",
        AgentGrade::Declines => "declines",
    }
}

/// Parse a wire `label` back into an [`AgentGrade`]. Inverse of
/// [`grade_label`]. Returns `None` for any unknown label so callers
/// can treat malformed input as a cache miss rather than a hard fail.
fn grade_from_label(label: &str) -> Option<AgentGrade> {
    Some(match label {
        "strong" => AgentGrade::Strong,
        "moderate" => AgentGrade::Moderate,
        "weak" => AgentGrade::Weak,
        "declines" => AgentGrade::Declines,
        _ => return None,
    })
}

/// Parse the one-line grade header `<TRANSCRIPT_FRAME_PREFIX><variant>\n`
/// and return `(grade, body_without_header)`. Returns `None` on
/// malformed input so the read path treats it as a miss.
///
/// Public so PR-4+ subscribers can validate transcript framing
/// symmetrically with [`frame_transcript_with_grade`].
pub fn parse_transcript_grade(transcript_bytes: &[u8]) -> Option<(AgentGrade, Vec<u8>)> {
    let bytes = transcript_bytes.strip_prefix(TRANSCRIPT_FRAME_PREFIX)?;
    let newline = bytes.iter().position(|&b| b == b'\n')?;
    let grade_str = std::str::from_utf8(&bytes[..newline]).ok()?;
    let grade = grade_from_label(grade_str)?;
    Some((grade, bytes[newline + 1..].to_vec()))
}

/// Inverse of [`parse_transcript_grade`] — frames a transcript body
/// with the grade header so the read path can recover the grade.
/// Public so PR-4+ (the runtime) can produce conformant transcripts.
pub fn frame_transcript_with_grade(grade: &AgentGrade, body: &[u8]) -> Vec<u8> {
    let label = grade_label(grade);
    // PREFIX + label + '\n' + body. The `+ 1` is the trailing newline.
    let mut out = Vec::with_capacity(TRANSCRIPT_FRAME_PREFIX.len() + label.len() + 1 + body.len());
    out.extend_from_slice(TRANSCRIPT_FRAME_PREFIX);
    out.extend_from_slice(label.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(body);
    out
}

mod hex {
    /// Lowercase hex encoding of a byte slice. Inlined here to avoid
    /// pulling the `hex` crate just for this one call site; the cache
    /// key is a fixed 32-byte SHA, so we know the output length and
    /// the formatting is trivial.
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let bytes = bytes.as_ref();
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
        out
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

    // ----- Phase 7 PR-2: multi-shot transcript cache ---------------

    fn agent_fp(transport: &str, target_sha: [u8; 32]) -> AgentInputFingerprint {
        AgentInputFingerprint {
            stage_id: "L3".to_string(),
            agent_id: "classifier".to_string(),
            agent_version: "v1".to_string(),
            prompt_template_sha: [3u8; 32],
            tool_catalog_sha: [4u8; 32],
            model_id: "claude-opus-4".to_string(),
            backend_version: "v1".to_string(),
            transport_flavour: transport.to_string(),
            target_input_shas: vec![target_sha],
            iteration_number: 0,
            prior_model_sha: None,
        }
    }

    fn agent_result_ok() -> AgentResult {
        let transcript = frame_transcript_with_grade(&AgentGrade::Strong, b"sample transcript");
        AgentResult::new(
            transcript,
            br#"{"kind":"library"}"#.to_vec(),
            AgentGrade::Strong,
        )
    }

    #[test]
    fn agent_cache_key_includes_transport_flavour() {
        // Same fingerprint differing only in transport_flavour must
        // hash to a different cache key — the spine relies on this so
        // switching transports between runs invalidates the cache
        // cleanly.
        let target = [9u8; 32];
        let a = agent_fp("claude_code", target).to_cache_key();
        let b = agent_fp("codex", target).to_cache_key();
        assert_ne!(
            a, b,
            "transport_flavour must contribute to the cache key (claude_code vs codex)"
        );

        // Sanity: same fp twice produces the same key.
        let c = agent_fp("claude_code", target).to_cache_key();
        assert_eq!(a, c);
    }

    #[test]
    fn agent_cache_evicts_on_recorded_fingerprint_input_sha_mismatch() {
        // Seed the cache with a result whose recorded_inputs claim
        // path "src/lib.rs" hashes to sha [1; 32]. Then re-call with
        // a current_sha_fn that returns sha [2; 32]. The cache must
        // evict the entry and re-run compute.
        let (persistent, _dir) = temp_persistent();
        let cache = LlmResponseCache::new_with_persistent(persistent.clone());

        let target = [9u8; 32];
        let fp = agent_fp("claude_code", target);

        // Manually plant a pair on disk so we don't rely on the write
        // path for the seeding step. PR-4 will go through call_agent_cached,
        // but for this test we want to isolate the eviction path.
        let key = fp.to_cache_key();
        let transcript_path = agents_transcript_path(persistent.root(), Stage::L3, &key);
        let output_path = agents_output_path(persistent.root(), Stage::L3, &key);
        let r = agent_result_ok();
        atomic_write_pair(
            &transcript_path,
            &r.transcript_bytes,
            &output_path,
            &r.output_bytes,
        )
        .unwrap();
        assert!(transcript_path.exists() && output_path.exists());

        let recorded = vec![FingerprintInputSpotCheck {
            path: "src/lib.rs".to_string(),
            recorded_sha: [1u8; 32],
        }];

        // Recompute closure returns a *new* result so we can verify
        // the eviction path actually re-ran.
        let computed = std::cell::Cell::new(0u32);
        let result = cache
            .call_agent_cached(
                Stage::L3,
                &fp,
                &recorded,
                |_| Some([2u8; 32]), // current sha differs from recorded
                AgentRequest::new(vec![]),
                |_req| {
                    computed.set(computed.get() + 1);
                    Ok(AgentResult::new(
                        frame_transcript_with_grade(&AgentGrade::Moderate, b"new-transcript"),
                        b"new-output".to_vec(),
                        AgentGrade::Moderate,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            computed.get(),
            1,
            "compute must be invoked exactly once on eviction"
        );
        assert_eq!(result.confidence_grade, AgentGrade::Moderate);
        assert_eq!(result.output_bytes, b"new-output");

        // The new pair landed.
        assert_eq!(
            std::fs::read(&output_path).unwrap(),
            b"new-output",
            "evicted entry must be replaced by recompute"
        );
    }

    #[test]
    fn agent_cache_atomic_pair_write_on_success() {
        // Cold call: compute succeeds, both files materialise.
        let (persistent, _dir) = temp_persistent();
        let cache = LlmResponseCache::new_with_persistent(persistent.clone());

        let target = [9u8; 32];
        let fp = agent_fp("claude_code", target);
        let key = fp.to_cache_key();

        let _ = cache
            .call_agent_cached(
                Stage::L3,
                &fp,
                &[],
                |_| None,
                AgentRequest::new(vec![]),
                |_| Ok(agent_result_ok()),
            )
            .unwrap();

        let transcript = agents_transcript_path(persistent.root(), Stage::L3, &key);
        let output = agents_output_path(persistent.root(), Stage::L3, &key);
        assert!(transcript.exists(), "transcript must land on success");
        assert!(output.exists(), "output must land on success");
        assert_eq!(std::fs::read(&output).unwrap(), b"{\"kind\":\"library\"}");
    }

    #[test]
    fn agent_cache_no_write_on_hard_fail() {
        // Compute returns Err — neither file must exist.
        let (persistent, _dir) = temp_persistent();
        let cache = LlmResponseCache::new_with_persistent(persistent.clone());

        let target = [9u8; 32];
        let fp = agent_fp("claude_code", target);
        let key = fp.to_cache_key();

        let err = cache
            .call_agent_cached(
                Stage::L3,
                &fp,
                &[],
                |_| None,
                AgentRequest::new(vec![]),
                |_| Err(AgentError::BackendFailed("simulated".into())),
            )
            .unwrap_err();

        assert!(matches!(err, AgentError::BackendFailed(_)));

        let transcript = agents_transcript_path(persistent.root(), Stage::L3, &key);
        let output = agents_output_path(persistent.root(), Stage::L3, &key);
        assert!(
            !transcript.exists(),
            "transcript must NOT land on hard fail (recast §6.4)"
        );
        assert!(
            !output.exists(),
            "output must NOT land on hard fail (recast §6.4)"
        );
    }

    #[test]
    fn agent_cache_hit_short_circuits_compute_when_spot_check_clean() {
        // Sanity: when recorded_inputs match the current view, the
        // cached pair is returned and compute is NOT invoked.
        let (persistent, _dir) = temp_persistent();
        let cache = LlmResponseCache::new_with_persistent(persistent.clone());

        let target = [9u8; 32];
        let fp = agent_fp("claude_code", target);
        let key = fp.to_cache_key();
        let transcript_path = agents_transcript_path(persistent.root(), Stage::L3, &key);
        let output_path = agents_output_path(persistent.root(), Stage::L3, &key);

        let r = agent_result_ok();
        atomic_write_pair(
            &transcript_path,
            &r.transcript_bytes,
            &output_path,
            &r.output_bytes,
        )
        .unwrap();

        let recorded = vec![FingerprintInputSpotCheck {
            path: "src/lib.rs".to_string(),
            recorded_sha: [1u8; 32],
        }];

        let computed = std::cell::Cell::new(0u32);
        let result = cache
            .call_agent_cached(
                Stage::L3,
                &fp,
                &recorded,
                |_| Some([1u8; 32]), // matches recorded
                AgentRequest::new(vec![]),
                |_| {
                    computed.set(computed.get() + 1);
                    Ok(agent_result_ok())
                },
            )
            .unwrap();

        assert_eq!(
            computed.get(),
            0,
            "compute must NOT be invoked on a clean spot-check hit"
        );
        assert_eq!(result.confidence_grade, AgentGrade::Strong);
        assert_eq!(result.output_bytes, b"{\"kind\":\"library\"}");
    }

    #[test]
    fn agent_cache_half_pair_is_treated_as_miss_and_overwritten() {
        // Simulate the post-rename-a / pre-rename-b crash residue:
        // a `<sha>.transcript` exists, but the corresponding
        // `<sha>.output` does not. `read_agent_pair` must:
        //   1. return `None` (so `call_agent_cached` falls through to
        //      compute);
        //   2. emit a `tracing::warn!` for the asymmetric-present case
        //      (signal for downstream debugging);
        //   3. let the subsequent atomic-pair write replace the residue.
        let (persistent, _dir) = temp_persistent();
        let cache = LlmResponseCache::new_with_persistent(persistent.clone());

        let target = [9u8; 32];
        let fp = agent_fp("claude_code", target);
        let key = fp.to_cache_key();
        let transcript_path = agents_transcript_path(persistent.root(), Stage::L3, &key);
        let output_path = agents_output_path(persistent.root(), Stage::L3, &key);

        // Plant only the transcript half on disk. The directory must
        // exist for the write to land — `agents_transcript_path`
        // returns a path under `cache/agents/L3/`; create it here so
        // the bare `fs::write` succeeds.
        if let Some(parent) = transcript_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let framed = frame_transcript_with_grade(&AgentGrade::Strong, b"orphan transcript");
        std::fs::write(&transcript_path, &framed).unwrap();
        assert!(transcript_path.exists());
        assert!(!output_path.exists());

        let computed = std::cell::Cell::new(0u32);
        let result = cache
            .call_agent_cached(
                Stage::L3,
                &fp,
                &[],
                |_| None,
                AgentRequest::new(vec![]),
                |_| {
                    computed.set(computed.get() + 1);
                    Ok(agent_result_ok())
                },
            )
            .unwrap();

        assert_eq!(
            computed.get(),
            1,
            "half-pair residue must be treated as a miss and recomputed"
        );
        assert_eq!(result.confidence_grade, AgentGrade::Strong);

        // Atomic-pair write overwrites the residue: both files exist.
        assert!(transcript_path.exists());
        assert!(output_path.exists());
    }
}
