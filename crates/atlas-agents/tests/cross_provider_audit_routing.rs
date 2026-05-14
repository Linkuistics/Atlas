//! PR-4: cross-provider audit routing acceptance test (plan §4 Task 4.8,
//! brainstorm §7.5, decision row 5).
//!
//! Exercises the *real audit code path* — `call_agent` with a Weak-grade
//! producer fires Lane B, which materialises the auditor backend via
//! `for_provider(producer_provider.cross())` and runs the audit-prompt
//! round-trip end-to-end. Strengthens the prior Phase 7 PR-5 routing
//! check (which only asserted `AuditDegraded` fired on single-provider
//! config without exercising the real audit prompt).
//!
//! Three scenarios:
//!
//! 1. Anthropic producer → OpenAI auditor (real cross-provider).
//! 2. OpenAI producer → Anthropic auditor (symmetry).
//! 3. Single-provider config → `AuditDegraded` event fires + auditor
//!    runs against the producer's own backend (same-model fallback).
//!
//! Memory `feedback_cross_provider_llm_audit` is the durable framing —
//! cross-provider audit is the entire reason Lane B exists.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use atlas_agents::events::{AgentEvent, EventBus};
use atlas_agents::runtime::audit::Stage;
use atlas_agents::runtime::{AgentRequest, ContentSha, ForProviderFn};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{default_tool_catalog, AgentRuntime, Semaphores};
use atlas_engine::llm_cache::LlmResponseCache;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, Provider};
use serde_json::{json, Value};

/// Tiny capturing backend that records every conversation it saw and
/// returns a single canned response. The fingerprint's `model_id`
/// carries the test's "side" label so cross-provider routing
/// assertions can identify which backend the auditor closure landed
/// on.
struct LabelBackend {
    label: String,
    response: Value,
    conversations: Mutex<Vec<String>>,
    call_count: std::sync::atomic::AtomicU32,
}

impl LabelBackend {
    fn arc(label: impl Into<String>, response: Value) -> Arc<Self> {
        Arc::new(Self {
            label: label.into(),
            response,
            conversations: Mutex::new(Vec::new()),
            call_count: std::sync::atomic::AtomicU32::new(0),
        })
    }

    fn call_count(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn last_conversation(&self) -> String {
        self.conversations
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl LlmBackend for LabelBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation("LabelBackend is async-only".into()))
    }
    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // WI-1: agent-runtime requests carry the prompt under
        // `rendered_prompt`.
        let conversation: String = req
            .rendered_prompt
            .clone()
            .or_else(|| {
                req.inputs
                    .get("conversation")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        self.conversations.lock().unwrap().push(conversation);
        Ok(self.response.clone())
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

fn text_block(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }] })
}

/// Producer's response: claims Strong but emits no evidence_pointers,
/// so Lane A's evidence-floor clamps to Declines → Lane B fires.
fn weak_classify_response() -> Value {
    text_block(concat!(
        "```yaml\n",
        "schema_version: 1\n",
        "kind: \"library\"\n",
        "language: \"rust\"\n",
        "lifecycle: \"active\"\n",
        "subsystem: \"agents\"\n",
        "child_component_ids: []\n",
        "candidates_considered: []\n",
        "evidence_pointers: []\n",
        "confidence_grade: \"strong\"\n",
        "rationale: \"baseline\"\n",
        "```\n"
    ))
}

fn audit_accept_response() -> Value {
    text_block(concat!(
        "```yaml\n",
        "verdict: accept\n",
        "reason: |\n",
        "  cross-provider auditor accepts; transcript inspected.\n",
        "```\n"
    ))
}

/// A substring-aware test backend. Returns `audit_resp` when the
/// conversation contains "You are an auditor"; otherwise returns
/// `producer_resp`.
struct SubstringBackend {
    label: String,
    producer_resp: Value,
    audit_resp: Value,
    audit_calls: std::sync::atomic::AtomicU32,
    producer_calls: std::sync::atomic::AtomicU32,
}

impl SubstringBackend {
    fn arc(label: impl Into<String>, producer_resp: Value, audit_resp: Value) -> Arc<Self> {
        Arc::new(Self {
            label: label.into(),
            producer_resp,
            audit_resp,
            audit_calls: std::sync::atomic::AtomicU32::new(0),
            producer_calls: std::sync::atomic::AtomicU32::new(0),
        })
    }

    fn audit_calls(&self) -> u32 {
        self.audit_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn producer_calls(&self) -> u32 {
        self.producer_calls
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmBackend for SubstringBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation(
            "SubstringBackend is async-only".into(),
        ))
    }
    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        // WI-1: agent-runtime requests carry the prompt under
        // `rendered_prompt`.
        let conversation: &str = req
            .rendered_prompt
            .as_deref()
            .or_else(|| req.inputs.get("conversation").and_then(Value::as_str))
            .unwrap_or("");
        if conversation.contains("You are an auditor") {
            self.audit_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.audit_resp.clone())
        } else {
            self.producer_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.producer_resp.clone())
        }
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

fn classify_request(target_id: impl Into<String>, transport: TransportFlavour) -> AgentRequest {
    let target_id = target_id.into();
    let candidate_ids: HashSet<String> = std::iter::once(target_id.clone()).collect();
    AgentRequest {
        stage: Stage::Classify,
        target_id: target_id.clone(),
        iteration: 1,
        transport,
        initial_prompt: format!(
            "You are Atlas's classify agent. Classify component `{target_id}`.\n"
        ),
        fingerprint_inputs: Vec::new(),
        candidate_ids,
        prior_model_sha: Some(ContentSha("0".repeat(64))),
        lane_b_revisions: 0,
    }
}

fn build_runtime(
    producer: Arc<dyn LlmBackend>,
    for_provider: Option<Arc<ForProviderFn>>,
    transport: TransportFlavour,
    audit_dir: std::path::PathBuf,
) -> AgentRuntime {
    AgentRuntime {
        backend_router: producer,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: Arc::new(EventBus::new(1024)),
        semaphores: Semaphores::defaults(),
        default_transport: transport,
        default_max_steps: 4,
        max_iterations: 1,
        for_provider,
        mcp_server: None,
        audit_dir,
    }
}

#[tokio::test]
async fn anthropic_producer_routes_to_openai_auditor_via_for_provider() {
    let audit_tmp = tempfile::tempdir().unwrap();

    let producer = LabelBackend::arc("anthropic-producer", weak_classify_response());
    let auditor = LabelBackend::arc("openai-auditor", audit_accept_response());

    let auditor_for_closure = auditor.clone();
    let for_provider: Arc<ForProviderFn> = Arc::new(move |p: Provider| {
        if p == Provider::OpenAi {
            Some(auditor_for_closure.clone() as Arc<dyn LlmBackend>)
        } else {
            None
        }
    });

    let runtime = build_runtime(
        producer.clone() as Arc<dyn LlmBackend>,
        Some(for_provider),
        TransportFlavour::HttpAnthropic,
        audit_tmp.path().to_path_buf(),
    );

    let mut rx = runtime.event_bus.subscribe();
    let result = runtime
        .call_agent(classify_request("foo", TransportFlavour::HttpAnthropic))
        .await;
    assert!(
        result.is_ok(),
        "cross-provider audit must complete on a valid auditor response; got {result:?}"
    );

    // Producer called once (initial classify); auditor called once
    // (real audit code path against the cross-provider backend).
    assert_eq!(producer.call_count(), 1);
    assert_eq!(
        auditor.call_count(),
        1,
        "auditor backend must actually run on Weak/Declines grade — \
         this is the real audit code path, not the stub-returns-Accept \
         shortcircuit"
    );

    // The auditor saw the audit prompt body.
    let last = auditor.last_conversation();
    assert!(
        last.contains("You are an auditor"),
        "auditor must receive the audit-prompt body"
    );
    assert!(
        last.contains("anthropic"),
        "audit-prompt body must label the producer as anthropic"
    );
    assert!(
        last.contains("openai"),
        "audit-prompt body must label the auditor as openai"
    );

    // AuditFire emits with auditor_provider="openai" — proving the
    // cross-provider lookup hit the OpenAI side, not degraded-mode
    // fallback.
    let mut saw_audit_fire_openai = false;
    let mut saw_audit_degraded = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::AuditFire {
                auditor_provider, ..
            } => {
                if auditor_provider == "openai" {
                    saw_audit_fire_openai = true;
                }
            }
            AgentEvent::AuditDegraded { .. } => saw_audit_degraded = true,
            _ => {}
        }
    }
    assert!(
        saw_audit_fire_openai,
        "AuditFire event must carry auditor_provider=openai on the \
         cross-provider happy path"
    );
    assert!(
        !saw_audit_degraded,
        "AuditDegraded must NOT fire when the cross-provider lookup \
         succeeds"
    );
}

#[tokio::test]
async fn openai_producer_routes_to_anthropic_auditor_symmetry() {
    let audit_tmp = tempfile::tempdir().unwrap();

    let producer = LabelBackend::arc("openai-producer", weak_classify_response());
    let auditor = LabelBackend::arc("anthropic-auditor", audit_accept_response());

    let auditor_for_closure = auditor.clone();
    let for_provider: Arc<ForProviderFn> = Arc::new(move |p: Provider| {
        if p == Provider::Anthropic {
            Some(auditor_for_closure.clone() as Arc<dyn LlmBackend>)
        } else {
            None
        }
    });

    let runtime = build_runtime(
        producer.clone() as Arc<dyn LlmBackend>,
        Some(for_provider),
        TransportFlavour::HttpOpenai,
        audit_tmp.path().to_path_buf(),
    );

    let mut rx = runtime.event_bus.subscribe();
    let result = runtime
        .call_agent(classify_request("foo", TransportFlavour::HttpOpenai))
        .await;
    assert!(
        result.is_ok(),
        "symmetric cross-provider audit must succeed; got {result:?}"
    );

    assert_eq!(producer.call_count(), 1);
    assert_eq!(
        auditor.call_count(),
        1,
        "auditor (anthropic) must run on the symmetric path"
    );

    let last = auditor.last_conversation();
    assert!(last.contains("You are an auditor"));
    assert!(
        last.contains("producer is a openai"),
        "audit-prompt body must label the producer as openai on the \
         symmetric path; got: {last}"
    );

    let mut saw_audit_fire_anthropic = false;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::AuditFire {
            auditor_provider, ..
        } = ev
        {
            if auditor_provider == "anthropic" {
                saw_audit_fire_anthropic = true;
            }
        }
    }
    assert!(
        saw_audit_fire_anthropic,
        "AuditFire must carry auditor_provider=anthropic on the symmetric path"
    );
}

#[tokio::test]
async fn single_provider_config_emits_audit_degraded_and_runs_real_audit() {
    let audit_tmp = tempfile::tempdir().unwrap();

    // Single backend handles both producer + auditor; no for_provider
    // lookup means Lane B degrades to same-model.
    let backend = SubstringBackend::arc(
        "anthropic-only",
        weak_classify_response(),
        audit_accept_response(),
    );

    let runtime = build_runtime(
        backend.clone() as Arc<dyn LlmBackend>,
        None, // no cross-provider lookup
        TransportFlavour::HttpAnthropic,
        audit_tmp.path().to_path_buf(),
    );

    let mut rx = runtime.event_bus.subscribe();
    let result = runtime
        .call_agent(classify_request("foo", TransportFlavour::HttpAnthropic))
        .await;
    assert!(
        result.is_ok(),
        "single-provider degraded audit must still complete; got {result:?}"
    );

    // Backend was called twice: once for the producer prompt, once
    // for the audit prompt (degraded-mode fallback uses the same
    // backend).
    assert_eq!(
        backend.producer_calls(),
        1,
        "backend should serve exactly one producer call"
    );
    assert_eq!(
        backend.audit_calls(),
        1,
        "backend should serve exactly one audit call — this is the \
         REAL audit code path running on same-model fallback (not the \
         pre-PR-4 stub-returns-Accept shortcircuit)"
    );

    // AuditDegraded MUST fire on single-provider config.
    let mut saw_audit_degraded = false;
    let mut saw_audit_fire = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::AuditDegraded { reason } => {
                saw_audit_degraded = true;
                assert!(
                    reason.contains("single-provider"),
                    "AuditDegraded reason must explain why fallback fired"
                );
            }
            AgentEvent::AuditFire { .. } => saw_audit_fire = true,
            _ => {}
        }
    }
    assert!(
        saw_audit_degraded,
        "AuditDegraded must fire when for_provider returns None for the \
         cross-provider lookup"
    );
    assert!(
        saw_audit_fire,
        "AuditFire must still fire on the degraded path — Lane B runs, \
         just against the same model"
    );
}

#[tokio::test]
async fn cross_provider_path_persists_verdict_with_correct_provider_labels() {
    // On-disk verdict's `producer.provider` and `auditor.provider`
    // must reflect the actual providers used (anthropic + openai for
    // the cross-provider happy path). PR-5's intrinsic-metrics
    // aggregator reads these fields to bucket cold-token totals per
    // provider.
    let audit_tmp = tempfile::tempdir().unwrap();
    let audit_dir = audit_tmp.path().to_path_buf();

    let producer = LabelBackend::arc("anthropic-producer", weak_classify_response());
    let auditor = LabelBackend::arc("openai-auditor", audit_accept_response());
    let auditor_for_closure = auditor.clone();
    let for_provider: Arc<ForProviderFn> = Arc::new(move |p: Provider| {
        if p == Provider::OpenAi {
            Some(auditor_for_closure.clone() as Arc<dyn LlmBackend>)
        } else {
            None
        }
    });

    let runtime = build_runtime(
        producer as Arc<dyn LlmBackend>,
        Some(for_provider),
        TransportFlavour::HttpAnthropic,
        audit_dir.clone(),
    );
    let _ = runtime
        .call_agent(classify_request("foo", TransportFlavour::HttpAnthropic))
        .await
        .unwrap();

    let on_disk = audit_dir.join("classify").join("foo.yaml");
    assert!(on_disk.exists(), "on-disk verdict must land at {on_disk:?}");
    let yaml = std::fs::read_to_string(&on_disk).unwrap();
    // Both provider fields present; producer side anthropic, auditor
    // side openai.
    assert!(
        yaml.contains("provider: anthropic"),
        "producer.provider must be anthropic; got:\n{yaml}"
    );
    assert!(
        yaml.contains("provider: openai"),
        "auditor.provider must be openai; got:\n{yaml}"
    );
    // Model id is recorded too — PR-5 calibration buckets by model.
    assert!(yaml.contains("openai-auditor"));
    assert!(yaml.contains("anthropic-producer"));
}
