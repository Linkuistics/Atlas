//! Build the production `ClaudeCodeBackend` wired through a
//! `BudgetedBackend`, plus the fingerprint derivations Atlas stamps
//! into `components.yaml`.
//!
//! Tests bypass the ClaudeCode path by constructing a `TestBackend`
//! and passing it to [`crate::run_index`]; they still benefit from the
//! [`BudgetSentinel`] wrapper, which is the only reliable way to
//! observe budget exhaustion once L5/L6 have collapsed the error into
//! a "LLM call failed" surface note.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use atlas_engine::sha256_hex;
use atlas_llm::{
    default_token_estimator, BudgetedBackend, ClaudeCodeBackend, LlmBackend, LlmError,
    LlmFingerprint, LlmRequest, TokenCounter,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::prompts::{EMBEDDED_ONTOLOGY_YAML, EMBEDDED_PROMPTS};

/// Everything the CLI needs to keep alive for the duration of a run.
/// The tempdir holds the materialised prompts that `ClaudeCodeBackend`
/// reads; dropping the handle before `run_index` finishes removes the
/// prompts from under the backend.
pub struct BackendHandles {
    pub backend: Arc<dyn LlmBackend>,
    pub counter: Option<Arc<TokenCounter>>,
    pub sentinel: Arc<BudgetSentinel>,
    pub fingerprint: LlmFingerprint,
    pub prompts_dir: TempDir,
}

/// Sticky-flag wrapper: forwards every call to the inner backend and
/// records whether any call ever returned [`LlmError::BudgetExhausted`].
/// The pipeline checks the flag after each stage so a budget-exhausted
/// run can be failed loudly even though L5/L6 swallow the error into a
/// default record.
pub struct BudgetSentinel {
    inner: Arc<dyn LlmBackend>,
    exhausted: AtomicBool,
}

impl BudgetSentinel {
    pub fn new(inner: Arc<dyn LlmBackend>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            exhausted: AtomicBool::new(false),
        })
    }

    pub fn was_exhausted(&self) -> bool {
        self.exhausted.load(Ordering::Acquire)
    }
}

impl LlmBackend for BudgetSentinel {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        match self.inner.call(req) {
            Err(LlmError::BudgetExhausted { requested, remaining }) => {
                self.exhausted.store(true, Ordering::Release);
                Err(LlmError::BudgetExhausted { requested, remaining })
            }
            other => other,
        }
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.inner.fingerprint()
    }
}

/// Construct the production backend stack: materialise prompts to a
/// tempdir, construct `ClaudeCodeBackend` pointed at it, compute the
/// run-wide fingerprint inputs, and wrap in `BudgetedBackend` when
/// `budget` is `Some`.
pub fn build_production_backend(
    model_id: String,
    budget: Option<u64>,
) -> Result<BackendHandles> {
    let prompts_dir = TempDir::new()?;
    crate::prompts::materialise_to(prompts_dir.path())?;

    let template_sha = compute_template_sha();
    let ontology_sha = compute_ontology_sha();

    let inner = ClaudeCodeBackend::new(model_id.clone(), prompts_dir.path())?
        .with_fingerprint_inputs(template_sha, ontology_sha);
    let version_fingerprint = inner.fingerprint();
    let inner_arc: Arc<dyn LlmBackend> = Arc::new(inner);

    let (backend_after_budget, counter) = match budget {
        Some(ceiling) => {
            let counter = Arc::new(TokenCounter::new(ceiling));
            let backend: Arc<dyn LlmBackend> = Arc::new(BudgetedBackend::new(
                inner_arc,
                counter.clone(),
                default_token_estimator(),
            ));
            (backend, Some(counter))
        }
        None => (inner_arc, None),
    };
    let sentinel = BudgetSentinel::new(backend_after_budget);
    let backend: Arc<dyn LlmBackend> = sentinel.clone();

    Ok(BackendHandles {
        backend,
        counter,
        sentinel,
        fingerprint: version_fingerprint,
        prompts_dir,
    })
}

/// SHA-256 over the concatenation of the embedded prompt bodies in
/// `EMBEDDED_PROMPTS` order. Used as the run-wide `template_sha` on
/// `LlmFingerprint`. Changing any prompt bumps this and invalidates
/// every cached LLM response, per the engine's memoisation contract.
pub fn compute_template_sha() -> [u8; 32] {
    let mut hasher = Sha256::new();
    for (_, name, body) in EMBEDDED_PROMPTS {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hasher.update(body.as_bytes());
    }
    hasher.finalize().into()
}

pub fn compute_ontology_sha() -> [u8; 32] {
    Sha256::digest(EMBEDDED_ONTOLOGY_YAML.as_bytes()).into()
}

/// Per-prompt SHAs (lowercase hex) of the embedded prompt bodies,
/// keyed by the prompt id string used in `CacheFingerprints::prompt_shas`.
pub fn compute_prompt_shas() -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for (_, name, body) in EMBEDDED_PROMPTS {
        map.insert((*name).to_string(), sha256_hex(body.as_bytes()));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_sha_is_deterministic() {
        assert_eq!(compute_template_sha(), compute_template_sha());
    }

    #[test]
    fn prompt_shas_cover_every_embedded_prompt() {
        let map = compute_prompt_shas();
        for (_, name, _) in EMBEDDED_PROMPTS {
            assert!(map.contains_key(*name), "missing sha for {name}");
        }
        assert_eq!(map.len(), EMBEDDED_PROMPTS.len());
    }

    #[test]
    fn ontology_sha_changes_when_bytes_change() {
        let canonical = compute_ontology_sha();
        let alt: [u8; 32] = Sha256::digest(b"different bytes").into();
        assert_ne!(canonical, alt);
    }
}
