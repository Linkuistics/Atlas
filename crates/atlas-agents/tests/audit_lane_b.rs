//! PR-5 Lane B cross-provider audit acceptance tests
//! (plan §4 Task 5.6, brainstorm §2 row 11).
//!
//! Validates the producer→auditor provider mapping rule
//! (Anthropic↔OpenAI), the same-model fallback emission of
//! `AgentEvent::AuditDegraded`, and the skip-on-strong-grade gate.

use std::sync::Arc;

use atlas_agents::events::{AgentEvent, EventBus, Grade};
use atlas_agents::runtime::audit::lane_b::{lane_b_audit, AuditVerdict};
use atlas_agents::transport::Provider;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};

/// Tiny `LlmBackend` impl whose `fingerprint().model_id` is the
/// caller-supplied label, so tests can assert which backend was
/// handed to the audit closure.
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
    fn call(&self, _req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        Err(LlmError::Invocation("unused".into()))
    }
    async fn call_async(&self, _req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
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

/// Helper: build a `for_provider` closure that returns `auditor` when
/// queried for `auditor_provider`, and `None` otherwise.
fn for_provider_returning(
    auditor_provider: Provider,
    auditor: Arc<dyn LlmBackend>,
) -> impl Fn(Provider) -> Option<Arc<dyn LlmBackend>> + Send + Sync + 'static {
    move |p| {
        if p == auditor_provider {
            Some(auditor.clone())
        } else {
            None
        }
    }
}

#[tokio::test]
async fn lane_b_routes_anthropic_producer_to_openai_auditor() {
    let producer = LabelBackend::arc("anthropic-producer");
    let auditor = LabelBackend::arc("openai-auditor");
    let bus = EventBus::new(32);
    let for_provider = for_provider_returning(Provider::OpenAi, auditor.clone());

    let observed_auditor = Arc::new(std::sync::Mutex::new(String::new()));
    let observed_auditor_clone = observed_auditor.clone();

    let verdict = lane_b_audit(
        &bus,
        "classify::foo#i1",
        &Grade::Weak,
        Provider::Anthropic,
        &producer,
        Some(&for_provider),
        move |chosen_backend| {
            let observed = observed_auditor_clone.clone();
            async move {
                *observed.lock().unwrap() = chosen_backend.fingerprint().model_id;
                AuditVerdict::Accept
            }
        },
    )
    .await;
    assert!(matches!(verdict, AuditVerdict::Accept));
    assert_eq!(
        *observed_auditor.lock().unwrap(),
        "openai-auditor",
        "anthropic producer must route to openai auditor"
    );
}

#[tokio::test]
async fn lane_b_routes_openai_producer_to_anthropic_auditor() {
    let producer = LabelBackend::arc("openai-producer");
    let auditor = LabelBackend::arc("anthropic-auditor");
    let bus = EventBus::new(32);
    let for_provider = for_provider_returning(Provider::Anthropic, auditor.clone());

    let observed_auditor = Arc::new(std::sync::Mutex::new(String::new()));
    let observed_auditor_clone = observed_auditor.clone();

    let verdict = lane_b_audit(
        &bus,
        "classify::foo#i1",
        &Grade::Weak,
        Provider::OpenAi,
        &producer,
        Some(&for_provider),
        move |chosen_backend| {
            let observed = observed_auditor_clone.clone();
            async move {
                *observed.lock().unwrap() = chosen_backend.fingerprint().model_id;
                AuditVerdict::Accept
            }
        },
    )
    .await;
    assert!(matches!(verdict, AuditVerdict::Accept));
    assert_eq!(
        *observed_auditor.lock().unwrap(),
        "anthropic-auditor",
        "openai producer must route to anthropic auditor"
    );
}

#[tokio::test]
async fn lane_b_falls_back_to_same_model_with_audit_degraded_warning() {
    let producer = LabelBackend::arc("anthropic-only-producer");
    let bus = EventBus::new(32);
    let mut rx = bus.subscribe();

    let observed_auditor = Arc::new(std::sync::Mutex::new(String::new()));
    let observed_auditor_clone = observed_auditor.clone();

    let verdict = lane_b_audit(
        &bus,
        "classify::foo#i1",
        &Grade::Weak,
        Provider::Anthropic,
        &producer,
        // No for_provider closure: single-provider config.
        None,
        move |chosen_backend| {
            let observed = observed_auditor_clone.clone();
            async move {
                *observed.lock().unwrap() = chosen_backend.fingerprint().model_id;
                AuditVerdict::Accept
            }
        },
    )
    .await;
    // The verdict surfaces as `Degraded(Accept)` so the caller can
    // distinguish a cross-provider Accept from a same-model Accept.
    match &verdict {
        AuditVerdict::Degraded(inner) => assert!(matches!(**inner, AuditVerdict::Accept)),
        other => panic!("expected Degraded wrapper, got {other:?}"),
    }
    assert!(verdict.accepted());
    assert_eq!(
        *observed_auditor.lock().unwrap(),
        "anthropic-only-producer",
        "same-model fallback must hand the producer's own backend to the audit closure"
    );

    // Drain the bus and verify the AuditDegraded event landed.
    let mut degraded_count = 0u32;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, AgentEvent::AuditDegraded { .. }) {
            degraded_count += 1;
        }
    }
    assert_eq!(
        degraded_count, 1,
        "exactly one AuditDegraded event must fire on the single-provider fallback path"
    );
}

#[tokio::test]
async fn lane_b_skipped_on_strong_confidence() {
    let producer = LabelBackend::arc("anthropic-producer");
    let bus = EventBus::new(32);
    let mut rx = bus.subscribe();

    let invoked = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let invoked_clone = invoked.clone();

    let verdict = lane_b_audit(
        &bus,
        "classify::foo#i1",
        &Grade::Strong,
        Provider::Anthropic,
        &producer,
        None,
        move |_chosen_backend| {
            let invoked = invoked_clone.clone();
            async move {
                invoked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                AuditVerdict::Accept
            }
        },
    )
    .await;
    assert!(matches!(verdict, AuditVerdict::Skipped));
    assert_eq!(
        invoked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "audit closure must NOT be invoked on the skip path"
    );
    // No AuditFire / AuditVerdict / AuditDegraded events on the skip
    // path.
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::AuditFire { .. }
            | AgentEvent::AuditVerdict { .. }
            | AgentEvent::AuditDegraded { .. } => {
                panic!("no audit events should fire on the skip path; got {ev:?}");
            }
            _ => {}
        }
    }
}
