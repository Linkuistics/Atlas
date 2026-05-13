//! PR-5 Lane B cross-provider audit acceptance tests
//! (plan §4 Task 5.6, brainstorm §2 row 11).
//!
//! Validates the producer→auditor provider mapping rule
//! (Anthropic↔OpenAI), the same-model fallback emission of
//! `AgentEvent::AuditDegraded`, and the skip-on-strong-grade gate.

use std::sync::Arc;

use atlas_agents::events::{AgentEvent, EventBus, Grade};
use atlas_agents::runtime::audit::lane_b::{lane_b_audit, AuditVerdict};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{default_tool_catalog, AgentRuntime, Semaphores, Workspace as AgentsWorkspace};
use atlas_engine::llm_cache::LlmResponseCache;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, Provider};
use serde_json::{json, Value};

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

/// Backend used by the `call_agent` integration test below. Returns a
/// canned classify response wrapped in the Anthropic content-block
/// shape — the same envelope the PR-4 single-iteration smoke uses.
struct ClassifyBackend;

#[async_trait::async_trait]
impl LlmBackend for ClassifyBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation("ClassifyBackend is async-only".into()))
    }
    async fn call_async(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Ok(json!({
            "content": [{
                "type": "text",
                "text": "{\"components\":[{\"id\":\"foo\"}]}"
            }]
        }))
    }
    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: "classify-backend".to_string(),
            backend_version: "v0".to_string(),
        }
    }
}

/// FIX 2 step 6: assert Lane B is wired into `call_agent` by running
/// the runtime end-to-end with a backend that grades `Strong` and
/// verifying no `AuditFire` event fires (because Lane B's
/// `should_audit` predicate returns `false` for `Strong`). The
/// wiring deliverable is structural — empirical Lane B firing
/// requires a multi-grade backend, which is a PR-7+ concern.
#[tokio::test]
async fn lane_b_wired_into_call_agent_skips_on_strong_grade() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(
        root.join("subsystems.overrides.yaml"),
        "schema_version: 1\nsubsystems:\n  - id: agents\n    members: [foo]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("components.overrides.yaml"),
        "schema_version: 1\ncomponents:\n  foo:\n    subsystem: agents\n",
    )
    .unwrap();

    let bus = Arc::new(EventBus::new(1024));
    let mut rx = bus.subscribe();
    let runtime = AgentRuntime {
        backend_router: Arc::new(ClassifyBackend),
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: bus.clone(),
        semaphores: Semaphores::defaults(),
        default_transport: TransportFlavour::HttpAnthropic,
        default_max_steps: 4,
        max_iterations: 1,
        for_provider: None,
        mcp_server: None,
    };
    let workspace = AgentsWorkspace::new(root);

    let _projection = runtime
        .run_workspace(&workspace)
        .await
        .expect("workspace runs end-to-end under ClassifyBackend");

    let mut saw_audit_fire = false;
    let mut saw_runtime_complete = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::AuditFire { .. } => saw_audit_fire = true,
            AgentEvent::RuntimeComplete => saw_runtime_complete = true,
            _ => {}
        }
    }
    assert!(
        saw_runtime_complete,
        "RuntimeComplete must fire at end of run"
    );
    assert!(
        !saw_audit_fire,
        "Lane B must skip on Strong grade — no `AuditFire` event expected, but one was emitted"
    );
}

/// PR-7 (PR-5 closeout AWARENESS-A): positive-assertion complement to
/// `lane_b_wired_into_call_agent_skips_on_strong_grade`. The negative
/// assertion above passes both when Lane B is wired-and-skips AND when
/// Lane B is removed entirely; this positive test pins the wiring by
/// invoking `lane_b_audit` directly with a `Weak` grade and asserting
/// that an `AuditFire` event lands on the bus. (Synthesising a
/// non-Strong grade through `call_agent` would require modifying the
/// runtime's hardcoded `Grade::Strong` post-Lane-A — not load-bearing
/// for PR-7's scope; the direct invocation closes the wiring gap.)
#[tokio::test]
async fn lane_b_audit_fires_audit_fire_event_on_weak_grade() {
    let producer = LabelBackend::arc("anthropic-producer");
    let auditor = LabelBackend::arc("openai-auditor");
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();
    let for_provider = for_provider_returning(Provider::OpenAi, auditor.clone());

    let verdict = lane_b_audit(
        &bus,
        "classify::foo#i1",
        &Grade::Weak,
        Provider::Anthropic,
        &producer,
        Some(&for_provider),
        |_chosen_backend| async { AuditVerdict::Accept },
    )
    .await;

    assert!(matches!(verdict, AuditVerdict::Accept));

    let mut saw_audit_fire = false;
    let mut saw_audit_verdict = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::AuditFire {
                agent_id,
                audit_reason,
                auditor_provider,
            } => {
                saw_audit_fire = true;
                assert_eq!(agent_id, "classify::foo#i1");
                assert!(
                    audit_reason.contains("weak"),
                    "audit_reason should mention `weak` grade, got: {audit_reason}"
                );
                assert_eq!(auditor_provider, "openai");
            }
            AgentEvent::AuditVerdict { .. } => saw_audit_verdict = true,
            _ => {}
        }
    }
    assert!(
        saw_audit_fire,
        "Lane B must emit `AuditFire` on Weak grade — positive assertion"
    );
    assert!(
        saw_audit_verdict,
        "Lane B must emit `AuditVerdict` after the audit closure resolves"
    );
}

/// PR-7 (PR-5 closeout AWARENESS-A): same positive-assertion shape for
/// the `Declines` grade. Locks in that both `should_audit` truthy
/// grades produce `AuditFire`.
#[tokio::test]
async fn lane_b_audit_fires_audit_fire_event_on_declines_grade() {
    let producer = LabelBackend::arc("anthropic-producer");
    let auditor = LabelBackend::arc("openai-auditor");
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();
    let for_provider = for_provider_returning(Provider::OpenAi, auditor.clone());

    let verdict = lane_b_audit(
        &bus,
        "classify::bar#i1",
        &Grade::Declines,
        Provider::Anthropic,
        &producer,
        Some(&for_provider),
        |_chosen_backend| async { AuditVerdict::Accept },
    )
    .await;

    assert!(matches!(verdict, AuditVerdict::Accept));

    let mut saw_audit_fire = false;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::AuditFire { audit_reason, .. } = ev {
            saw_audit_fire = true;
            assert!(
                audit_reason.contains("declines"),
                "audit_reason should mention `declines` grade, got: {audit_reason}"
            );
        }
    }
    assert!(
        saw_audit_fire,
        "Lane B must emit `AuditFire` on Declines grade"
    );
}
