//! Build the production LLM backend stack driven by `.atlas/config.yaml`,
//! plus the fingerprint derivations Atlas stamps into `components.yaml`.
//!
//! Tests bypass this path by constructing a `TestBackend` directly and
//! passing it to [`crate::run_index`]; they still benefit from the
//! [`BudgetSentinel`] wrapper, which is the only reliable way to
//! observe budget exhaustion or backend-setup failure once L5/L6 have
//! collapsed the underlying error into a "LLM call failed" surface
//! note.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use atlas_engine::sha256_hex;
use atlas_llm::{
    default_token_estimator, BudgetedBackend, LlmBackend, LlmError, LlmFingerprint, LlmRequest,
    TokenCounter,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::prompts::{EMBEDDED_ONTOLOGY_YAML, EMBEDDED_PROMPTS};

/// Everything the CLI needs to keep alive for the duration of a run.
pub struct BackendHandles {
    pub backend: Arc<dyn LlmBackend>,
    pub counter: Option<Arc<TokenCounter>>,
    pub sentinel: Arc<BudgetSentinel>,
    pub fingerprint: LlmFingerprint,
    pub prompts_dir: TempDir,
}

/// Sticky-flag wrapper: forwards every call to the inner backend and
/// records whether any call ever returned [`LlmError::BudgetExhausted`]
/// or [`LlmError::Setup`]. Both are terminal conditions for the run —
/// budget exhaustion means subsequent calls would also fail because the
/// counter is drained; setup failure means the configuration is broken
/// and every call will hit the same root cause. Recording them at the
/// top of the backend stack lets the pipeline check after the
/// fixedpoint without threading a custom error type through every
/// L-layer.
pub struct BudgetSentinel {
    inner: Arc<dyn LlmBackend>,
    exhausted: AtomicBool,
    setup_failed: AtomicBool,
    setup_message: Mutex<Option<String>>,
}

impl BudgetSentinel {
    pub fn new(inner: Arc<dyn LlmBackend>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            exhausted: AtomicBool::new(false),
            setup_failed: AtomicBool::new(false),
            setup_message: Mutex::new(None),
        })
    }

    pub fn was_exhausted(&self) -> bool {
        self.exhausted.load(Ordering::Acquire)
    }

    pub fn was_setup_failed(&self) -> bool {
        self.setup_failed.load(Ordering::Acquire)
    }

    /// First Setup-error message observed by the sentinel, if any. The
    /// pipeline propagates this verbatim so the user sees the root cause
    /// rather than a generic "backend setup failed" line.
    pub fn first_setup_message(&self) -> Option<String> {
        self.setup_message
            .lock()
            .expect("setup_message poisoned")
            .clone()
    }
}

impl LlmBackend for BudgetSentinel {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        match self.inner.call(req) {
            Err(LlmError::BudgetExhausted {
                requested,
                remaining,
            }) => {
                self.exhausted.store(true, Ordering::Release);
                Err(LlmError::BudgetExhausted {
                    requested,
                    remaining,
                })
            }
            Err(LlmError::Setup(msg)) => {
                self.setup_failed.store(true, Ordering::Release);
                let mut slot = self.setup_message.lock().expect("setup_message poisoned");
                if slot.is_none() {
                    *slot = Some(msg.clone());
                }
                Err(LlmError::Setup(msg))
            }
            other => other,
        }
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.inner.fingerprint()
    }

    fn supports_filesystem_tools(&self) -> bool {
        self.inner.supports_filesystem_tools()
    }
}

/// Construct the production backend stack from an `AtlasConfig`.
pub fn build_production_backend_with_counter(
    config: &atlas_llm::AtlasConfig,
    counter: Option<Arc<TokenCounter>>,
    observer: Option<Arc<dyn atlas_llm::AgentObserver>>,
) -> Result<BackendHandles> {
    let prompts_dir = TempDir::new()?;
    crate::prompts::materialise_to(prompts_dir.path())?;

    let template_sha = compute_template_sha();
    let ontology_sha = compute_ontology_sha();

    let router = atlas_llm::BackendRouter::new(
        config,
        prompts_dir.path(),
        template_sha,
        ontology_sha,
        observer,
    )
    .map_err(|e| anyhow::anyhow!("failed to build backend: {e}"))?;
    let fingerprint = router.fingerprint();
    let inner: Arc<dyn LlmBackend> = Arc::new(router);

    let backend_after_budget: Arc<dyn LlmBackend> = match counter.as_ref() {
        Some(c) => Arc::new(BudgetedBackend::new(
            inner,
            c.clone(),
            default_token_estimator(),
        )),
        None => inner,
    };
    let sentinel = BudgetSentinel::new(backend_after_budget);
    let backend: Arc<dyn LlmBackend> = sentinel.clone();

    Ok(BackendHandles {
        backend,
        counter,
        sentinel,
        fingerprint,
        prompts_dir,
    })
}

/// SHA-256 over the concatenation of the embedded prompt bodies.
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

/// Per-prompt SHAs (lowercase hex) of the embedded prompt bodies.
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
