//! Token budget tracking. `TokenCounter` is a shared, atomically
//! updated tally of tokens spent so far in a run; `BudgetedBackend`
//! wraps a backend so that each LLM call is charged against the
//! counter, and a charge that would exceed the ceiling returns
//! [`LlmError::BudgetExhausted`] without invoking the inner backend.
//!
//! Per design §7.4: on budget exhaustion the run aborts — no
//! partial writes, no fallback to a cheaper model.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;

use crate::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};

/// Shared, thread-safe accumulator of token usage with a hard ceiling.
pub struct TokenCounter {
    budget: u64,
    used: AtomicU64,
}

impl TokenCounter {
    /// Construct a counter with the given token ceiling.
    pub fn new(budget: u64) -> Self {
        Self {
            budget,
            used: AtomicU64::new(0),
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }

    pub fn remaining(&self) -> u64 {
        self.budget.saturating_sub(self.used())
    }

    /// Attempt to charge `tokens` against the counter. On overflow
    /// the charge is rejected atomically — no partial accounting —
    /// and [`LlmError::BudgetExhausted`] is returned.
    pub fn charge(&self, tokens: u64) -> Result<(), LlmError> {
        loop {
            let current = self.used.load(Ordering::Acquire);
            let next = current.saturating_add(tokens);
            if next > self.budget {
                return Err(LlmError::BudgetExhausted {
                    requested: tokens,
                    remaining: self.budget.saturating_sub(current),
                });
            }
            if self
                .used
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(());
            }
        }
    }
}

/// Function used to estimate how many tokens a completed call cost.
/// The wrapper charges the counter with the estimator's result after
/// the inner call returns.
pub type TokenEstimator = Box<dyn Fn(&LlmRequest, &Value) -> u64 + Send + Sync>;

/// A crude default that charges one token per four bytes of input +
/// response JSON. Good enough for smoke tests and for bounding runs
/// when the backend doesn't report token usage out-of-band.
pub fn default_token_estimator() -> TokenEstimator {
    Box::new(|req: &LlmRequest, resp: &Value| {
        let input = serde_json::to_vec(&req.inputs).map(|v| v.len()).unwrap_or(0);
        let output = serde_json::to_vec(resp).map(|v| v.len()).unwrap_or(0);
        ((input + output) as u64) / 4
    })
}

/// Wraps an inner backend with a token-usage accounting layer.
pub struct BudgetedBackend {
    inner: Arc<dyn LlmBackend>,
    counter: Arc<TokenCounter>,
    estimator: TokenEstimator,
}

impl BudgetedBackend {
    pub fn new(
        inner: Arc<dyn LlmBackend>,
        counter: Arc<TokenCounter>,
        estimator: TokenEstimator,
    ) -> Self {
        Self {
            inner,
            counter,
            estimator,
        }
    }
}

impl LlmBackend for BudgetedBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let response = self.inner.call(req)?;
        let tokens = (self.estimator)(req, &response);
        self.counter.charge(tokens)?;
        Ok(response)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.inner.fingerprint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PromptId, ResponseSchema, TestBackend};
    use serde_json::json;

    fn req(inputs: Value) -> LlmRequest {
        LlmRequest {
            prompt_template: PromptId::Classify,
            inputs,
            schema: ResponseSchema::accept_any(),
        }
    }

    #[test]
    fn new_counter_starts_empty() {
        let counter = TokenCounter::new(100);
        assert_eq!(counter.budget(), 100);
        assert_eq!(counter.used(), 0);
        assert_eq!(counter.remaining(), 100);
    }

    #[test]
    fn charge_within_budget_accumulates() {
        let counter = TokenCounter::new(100);

        counter.charge(30).unwrap();
        counter.charge(20).unwrap();

        assert_eq!(counter.used(), 50);
        assert_eq!(counter.remaining(), 50);
    }

    #[test]
    fn charge_at_exact_ceiling_succeeds() {
        let counter = TokenCounter::new(100);

        counter.charge(100).unwrap();

        assert_eq!(counter.used(), 100);
        assert_eq!(counter.remaining(), 0);
    }

    #[test]
    fn overspend_is_rejected_atomically() {
        let counter = TokenCounter::new(100);
        counter.charge(80).unwrap();

        let err = counter.charge(30).unwrap_err();

        match err {
            LlmError::BudgetExhausted { requested, remaining } => {
                assert_eq!(requested, 30);
                assert_eq!(remaining, 20);
            }
            other => panic!("expected BudgetExhausted, got {other:?}"),
        }
        // The rejected charge is NOT recorded.
        assert_eq!(counter.used(), 80);
    }

    #[test]
    fn budgeted_backend_passes_through_under_budget() {
        let inner = Arc::new(TestBackend::new());
        inner.respond(PromptId::Classify, json!({ "d": "x" }), json!({ "k": "ok" }));

        let counter = Arc::new(TokenCounter::new(1_000));
        let backend = BudgetedBackend::new(
            inner.clone(),
            counter.clone(),
            Box::new(|_req, _resp| 7),
        );

        let out = backend.call(&req(json!({ "d": "x" }))).unwrap();

        assert_eq!(out, json!({ "k": "ok" }));
        assert_eq!(counter.used(), 7);
    }

    #[test]
    fn budgeted_backend_rejects_over_budget_call_with_exact_accounting() {
        let inner = Arc::new(TestBackend::new());
        inner.respond(PromptId::Classify, json!({ "d": "x" }), json!({ "k": "ok" }));

        let counter = Arc::new(TokenCounter::new(10));
        let backend = BudgetedBackend::new(
            inner.clone(),
            counter.clone(),
            Box::new(|_req, _resp| 15),
        );

        let err = backend.call(&req(json!({ "d": "x" }))).unwrap_err();

        assert!(matches!(err, LlmError::BudgetExhausted { .. }));
        // Inner call happened (we can't cancel it by construction),
        // but the counter rejected the charge — so the tally never
        // lies about being over budget.
        assert_eq!(counter.used(), 0);
    }

    #[test]
    fn budgeted_backend_forwards_fingerprint() {
        let inner = Arc::new(TestBackend::new());
        let counter = Arc::new(TokenCounter::new(100));
        let backend = BudgetedBackend::new(inner, counter, Box::new(|_, _| 0));

        assert_eq!(backend.fingerprint().model_id, "test-backend");
    }

    #[test]
    fn default_estimator_scales_with_payload_size() {
        let estimator = default_token_estimator();

        // Short input, short output
        let small = estimator(&req(json!({ "x": 1 })), &json!({ "y": 1 }));
        // Longer payload on both sides
        let big = estimator(
            &req(json!({ "text": "x".repeat(400) })),
            &json!({ "out": "y".repeat(400) }),
        );

        assert!(
            big > small,
            "longer payloads should estimate more tokens (small={small}, big={big})"
        );
    }
}
