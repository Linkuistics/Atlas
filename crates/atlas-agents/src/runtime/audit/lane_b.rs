//! Lane B — cross-provider audit (recast §4.3, brainstorm §2 row 11).
//!
//! Lane B fires when an agent's `confidence_grade` is `Weak` or
//! `Declines`. The producer's output is handed to a *different
//! provider's* auditor (Anthropic-produced output is audited by an
//! OpenAI model; OpenAI-produced output is audited by an Anthropic
//! model). Cross-provider audit avoids the same-model
//! self-confirmation pathology where the same model rubber-stamps its
//! own dubious output.
//!
//! When the active configuration only has a single provider wired in,
//! Lane B falls back to a same-model auditor and emits
//! [`AgentEvent::AuditDegraded`] so subscribers can flag the
//! degradation. The single-provider fallback is *strictly less
//! valuable* than the cross-provider audit (the user feedback memory
//! `feedback_cross_provider_llm_audit` documents this rigorously), but
//! it's strictly better than no audit at all.
//!
//! # Retry budget
//!
//! Lane A may fire one retry; Lane B may fire one retry — for a
//! combined max of two retries per agent before
//! [`AgentError::LaneBFail`] (recast §4.3). The retry policy is
//! implemented in the calling site (PR-5 wires it from
//! [`super::super::AgentRuntime::call_agent`]); this module owns the
//! verdict computation, not the retry harness.

use std::sync::Arc;

use atlas_llm::{LlmBackend, Provider};

use crate::events::{AgentEvent, EventBus, Grade};

/// Lane B verdict shape returned by [`lane_b_audit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditVerdict {
    /// Audit passed; the producer's output stands.
    Accept,
    /// Audit failed; the runtime should request a revision (one retry)
    /// and re-audit. Carries the auditor's reason so the retry prompt
    /// can mention what to fix.
    RequestRevision(String),
    /// Audit hard-failed; surfaces as [`super::super::AgentError::LaneBFail`].
    HardFail(String),
    /// Audit was skipped because the producer's grade was strong enough
    /// that audit was unnecessary (`Grade::Strong` / `Grade::Moderate`).
    Skipped,
    /// Audit was performed against a same-model auditor (degraded)
    /// rather than a cross-provider auditor. Carries the underlying
    /// verdict so the caller can act on it. The companion
    /// `AuditDegraded` event has already been emitted.
    Degraded(Box<AuditVerdict>),
}

impl AuditVerdict {
    /// True iff the audit verdict accepts (or skipped) the producer's
    /// output. Used by the caller's retry harness as the "did the
    /// audit pass?" predicate.
    pub fn accepted(&self) -> bool {
        match self {
            AuditVerdict::Accept | AuditVerdict::Skipped => true,
            AuditVerdict::Degraded(inner) => inner.accepted(),
            AuditVerdict::RequestRevision(_) | AuditVerdict::HardFail(_) => false,
        }
    }
}

/// Compute the auditor provider for `producer_provider` per the
/// cross-provider mapping rule. Pure helper; tested in isolation.
pub fn auditor_provider_for(producer_provider: Provider) -> Provider {
    producer_provider.cross()
}

/// Should Lane B fire for a producer that emitted `grade`? Pure helper
/// so the runtime + tests agree on the predicate. PR-5: fires on
/// `Weak` or `Declines` (recast §4.3).
pub fn should_audit(grade: &Grade) -> bool {
    matches!(grade, Grade::Weak | Grade::Declines)
}

/// Decision shape for which auditor backend to use, returned by
/// [`select_auditor_backend`]. Lets the caller distinguish the
/// cross-provider happy path from the single-provider degraded path
/// before any actual auditor call happens — so the
/// `AuditDegraded` event fires deterministically before the audit
/// starts (cleaner than firing it after the verdict is known).
#[derive(Clone)]
pub enum AuditorChoice {
    /// Cross-provider auditor located via `for_provider`.
    CrossProvider {
        provider: Provider,
        backend: Arc<dyn LlmBackend>,
    },
    /// Single-provider fallback. The producer's own provider's backend
    /// is the auditor; `AuditDegraded` has been emitted on the
    /// containing bus.
    Degraded {
        provider: Provider,
        backend: Arc<dyn LlmBackend>,
    },
}

impl AuditorChoice {
    /// Underlying backend handle regardless of cross-provider vs.
    /// degraded.
    pub fn backend(&self) -> &Arc<dyn LlmBackend> {
        match self {
            AuditorChoice::CrossProvider { backend, .. }
            | AuditorChoice::Degraded { backend, .. } => backend,
        }
    }

    /// The provider that the auditor runs under (always the *auditor*
    /// provider, not the producer — but in the degraded case they
    /// coincide).
    pub fn provider(&self) -> Provider {
        match self {
            AuditorChoice::CrossProvider { provider, .. }
            | AuditorChoice::Degraded { provider, .. } => *provider,
        }
    }

    /// True iff this choice is the single-provider fallback.
    pub fn is_degraded(&self) -> bool {
        matches!(self, AuditorChoice::Degraded { .. })
    }
}

/// Pick the auditor backend for `producer_provider`. Calls
/// `for_provider(auditor_provider)` to look up the cross-provider
/// backend; on `None`, emits [`AgentEvent::AuditDegraded`] on `bus`
/// and falls back to the producer's own backend.
///
/// `producer_backend` is the same `Arc<dyn LlmBackend>` the runtime
/// used to compute the producer's output. The degraded path returns
/// this handle so the caller's audit invocation does not require
/// re-resolving the producer's backend.
pub fn select_auditor_backend(
    producer_provider: Provider,
    producer_backend: &Arc<dyn LlmBackend>,
    for_provider: Option<&(dyn Fn(Provider) -> Option<Arc<dyn LlmBackend>> + Send + Sync)>,
    bus: &EventBus,
) -> AuditorChoice {
    let auditor_provider = auditor_provider_for(producer_provider);
    let cross = for_provider.and_then(|f| f(auditor_provider));
    match cross {
        Some(backend) => AuditorChoice::CrossProvider {
            provider: auditor_provider,
            backend,
        },
        None => {
            bus.emit(AgentEvent::AuditDegraded {
                reason: format!(
                    "single-provider config: no {} backend for cross-provider audit of {} \
                     producer; falling back to same-model auditor",
                    provider_label(auditor_provider),
                    provider_label(producer_provider),
                ),
            });
            AuditorChoice::Degraded {
                provider: producer_provider,
                backend: producer_backend.clone(),
            }
        }
    }
}

/// Stable label string for a `Provider`. Used in event payloads and
/// for the cross-provider audit reason string.
pub fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "anthropic",
        Provider::OpenAi => "openai",
    }
}

/// Lane B audit entry point.
///
/// Returns `AuditVerdict::Skipped` when `producer_grade` is strong
/// enough to skip audit; otherwise picks the auditor backend via
/// [`select_auditor_backend`] and emits [`AgentEvent::AuditFire`] +
/// [`AgentEvent::AuditVerdict`] around the audit call.
///
/// `audit_fn` is the per-call auditor invocation: given the auditor
/// backend, it returns the verdict. The decoupling keeps this
/// function testable without spinning up a real `LlmBackend` —
/// tests inject a closure that returns a canned `AuditVerdict`.
///
/// `agent_id` is the producer's agent id, used as the event-payload
/// correlator so subscribers can stitch the audit events back to the
/// producer's `AgentStart`/`AgentComplete` pair.
pub async fn lane_b_audit<F, Fut>(
    bus: &EventBus,
    agent_id: &str,
    producer_grade: &Grade,
    producer_provider: Provider,
    producer_backend: &Arc<dyn LlmBackend>,
    for_provider: Option<&(dyn Fn(Provider) -> Option<Arc<dyn LlmBackend>> + Send + Sync)>,
    audit_fn: F,
) -> AuditVerdict
where
    F: FnOnce(AuditorChoice) -> Fut,
    Fut: std::future::Future<Output = AuditVerdict>,
{
    if !should_audit(producer_grade) {
        return AuditVerdict::Skipped;
    }
    let choice = select_auditor_backend(producer_provider, producer_backend, for_provider, bus);
    bus.emit(AgentEvent::AuditFire {
        agent_id: agent_id.to_string(),
        audit_reason: format!("grade={}", grade_label(producer_grade)),
        auditor_provider: provider_label(choice.provider()).to_string(),
    });
    let degraded = choice.is_degraded();
    let verdict = audit_fn(choice).await;
    let final_verdict = if degraded {
        AuditVerdict::Degraded(Box::new(verdict))
    } else {
        verdict
    };
    bus.emit(AgentEvent::AuditVerdict {
        agent_id: agent_id.to_string(),
        verdict: verdict_label(&final_verdict).to_string(),
    });
    final_verdict
}

/// Stable wire string for a `Grade`. Inverse of the engine's
/// `AgentGrade` label table.
fn grade_label(grade: &Grade) -> &'static str {
    match grade {
        Grade::Strong => "strong",
        Grade::Moderate => "moderate",
        Grade::Weak => "weak",
        Grade::Declines => "declines",
    }
}

/// Stable wire string for an `AuditVerdict`. Used in the
/// `AuditVerdict.verdict` event payload. The degraded wrapper is
/// surfaced via the `degraded:` prefix so subscribers can detect it
/// without unpacking.
fn verdict_label(verdict: &AuditVerdict) -> String {
    match verdict {
        AuditVerdict::Accept => "accept".to_string(),
        AuditVerdict::RequestRevision(_) => "request_revision".to_string(),
        AuditVerdict::HardFail(_) => "hard_fail".to_string(),
        AuditVerdict::Skipped => "skipped".to_string(),
        AuditVerdict::Degraded(inner) => format!("degraded:{}", verdict_label(inner)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_llm::{LlmError, LlmFingerprint, LlmRequest};
    use serde_json::Value;

    /// Minimal `LlmBackend` whose `fingerprint().model_id` carries the
    /// caller-supplied label, so tests can assert which backend a
    /// chooser picked.
    struct LabelBackend {
        label: String,
    }

    impl LabelBackend {
        fn arc(label: &str) -> Arc<dyn LlmBackend> {
            Arc::new(Self {
                label: label.to_string(),
            })
        }
    }

    #[async_trait::async_trait]
    impl LlmBackend for LabelBackend {
        fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
            Err(LlmError::Invocation("unused".into()))
        }
        async fn call_async(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
            Err(LlmError::Invocation("unused".into()))
        }
        fn fingerprint(&self) -> LlmFingerprint {
            LlmFingerprint {
                template_sha: [0u8; 32],
                ontology_sha: [0u8; 32],
                model_id: self.label.clone(),
                backend_version: "v0".to_string(),
            }
        }
    }

    #[test]
    fn auditor_provider_inverts_producer_provider() {
        assert_eq!(auditor_provider_for(Provider::Anthropic), Provider::OpenAi);
        assert_eq!(auditor_provider_for(Provider::OpenAi), Provider::Anthropic);
    }

    #[test]
    fn should_audit_fires_only_on_weak_or_declines() {
        assert!(!should_audit(&Grade::Strong));
        assert!(!should_audit(&Grade::Moderate));
        assert!(should_audit(&Grade::Weak));
        assert!(should_audit(&Grade::Declines));
    }

    #[test]
    fn select_auditor_picks_cross_provider_when_available() {
        let producer = LabelBackend::arc("anthropic-producer");
        let auditor = LabelBackend::arc("openai-auditor");
        let bus = EventBus::new(16);
        let for_provider = {
            let auditor = auditor.clone();
            move |p: Provider| {
                if p == Provider::OpenAi {
                    Some(auditor.clone())
                } else {
                    None
                }
            }
        };
        let choice =
            select_auditor_backend(Provider::Anthropic, &producer, Some(&for_provider), &bus);
        assert!(!choice.is_degraded(), "expected cross-provider choice");
        assert_eq!(choice.provider(), Provider::OpenAi);
        assert_eq!(choice.backend().fingerprint().model_id, "openai-auditor");
    }

    #[tokio::test]
    async fn select_auditor_degrades_when_for_provider_returns_none() {
        let producer = LabelBackend::arc("anthropic-producer");
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let for_provider = |_p: Provider| None;
        let choice =
            select_auditor_backend(Provider::Anthropic, &producer, Some(&for_provider), &bus);
        assert!(choice.is_degraded(), "expected degraded fallback");
        assert_eq!(choice.provider(), Provider::Anthropic);
        // Sanity check that AuditDegraded landed on the bus.
        let ev = rx.recv().await.expect("AuditDegraded should be emitted");
        match ev {
            AgentEvent::AuditDegraded { reason } => {
                assert!(reason.contains("single-provider"));
            }
            other => panic!("expected AuditDegraded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn select_auditor_degrades_when_for_provider_is_absent() {
        let producer = LabelBackend::arc("anthropic-producer");
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let choice = select_auditor_backend(Provider::Anthropic, &producer, None, &bus);
        assert!(choice.is_degraded());
        // AuditDegraded fires on this path too.
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, AgentEvent::AuditDegraded { .. }));
    }

    #[tokio::test]
    async fn lane_b_audit_skips_on_strong_grade() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let producer = LabelBackend::arc("anthropic-producer");
        let verdict = lane_b_audit(
            &bus,
            "classify::foo#i1",
            &Grade::Strong,
            Provider::Anthropic,
            &producer,
            None,
            |_| async { AuditVerdict::Accept },
        )
        .await;
        assert!(matches!(verdict, AuditVerdict::Skipped));
        // No events fire on the skip path.
        let recv = rx.try_recv();
        assert!(
            recv.is_err(),
            "no events should fire on skip path, got {recv:?}"
        );
    }

    #[tokio::test]
    async fn lane_b_audit_emits_audit_fire_and_verdict_on_weak_grade() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let producer = LabelBackend::arc("anthropic-producer");
        let auditor = LabelBackend::arc("openai-auditor");
        let for_provider = {
            let auditor = auditor.clone();
            move |p: Provider| {
                if p == Provider::OpenAi {
                    Some(auditor.clone())
                } else {
                    None
                }
            }
        };
        let verdict = lane_b_audit(
            &bus,
            "classify::foo#i1",
            &Grade::Weak,
            Provider::Anthropic,
            &producer,
            Some(&for_provider),
            |choice| async move {
                assert_eq!(choice.backend().fingerprint().model_id, "openai-auditor");
                AuditVerdict::Accept
            },
        )
        .await;
        assert!(matches!(verdict, AuditVerdict::Accept));

        let ev1 = rx.recv().await.unwrap();
        match ev1 {
            AgentEvent::AuditFire {
                agent_id,
                audit_reason,
                auditor_provider,
            } => {
                assert_eq!(agent_id, "classify::foo#i1");
                assert!(audit_reason.contains("weak"));
                assert_eq!(auditor_provider, "openai");
            }
            other => panic!("expected AuditFire, got {other:?}"),
        }
        let ev2 = rx.recv().await.unwrap();
        match ev2 {
            AgentEvent::AuditVerdict { verdict, .. } => assert_eq!(verdict, "accept"),
            other => panic!("expected AuditVerdict, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn lane_b_audit_wraps_verdict_in_degraded_when_for_provider_is_none() {
        let bus = EventBus::new(16);
        let producer = LabelBackend::arc("anthropic-producer");
        let verdict = lane_b_audit(
            &bus,
            "classify::foo#i1",
            &Grade::Declines,
            Provider::Anthropic,
            &producer,
            None,
            |_chosen| async { AuditVerdict::Accept },
        )
        .await;
        match &verdict {
            AuditVerdict::Degraded(inner) => assert!(matches!(**inner, AuditVerdict::Accept)),
            other => panic!("expected Degraded wrapper, got {other:?}"),
        }
        assert!(verdict.accepted());
    }
}
