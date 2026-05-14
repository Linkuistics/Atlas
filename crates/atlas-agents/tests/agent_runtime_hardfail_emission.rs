//! WI-2: HardFail event emission for backend errors in `call_agent`.
//!
//! Sprint PR-5 closeout note item 4 surfaced the gap: `RuntimeComplete`
//! fires correctly today, but per-agent `HardFail` events for backend
//! errors were swallowed by the bare `?` propagation inside
//! `run_tool_loop_with_lane_a` (producer-fail) and by the existing
//! `return AuditVerdict::HardFail(...)` inside `run_real_audit`
//! (auditor-fail). Without these emits the JSONL event-log produced by
//! `--log-events` carries no diagnostic record of *why* an agent
//! returned `Err`.
//!
//! Two test cases exercise the two emission sites:
//!
//!  - **Producer-fail:** the producer's backend errors on the very
//!    first `call_async`. Assert (1) `HardFail { error_kind: "backend",
//!    .. }` lands on the bus; (2) the propagated `Err` summary carries
//!    the backend's verbatim error text.
//!  - **Auditor-fail:** the producer's backend returns a Weak-grade
//!    classify YAML envelope (Lane A passes via evidence-floor clamp →
//!    Lane B fires); the auditor backend errors. Assert (1) a
//!    `HardFail { error_kind: "audit_backend", .. }` lands on the bus
//!    carrying the auditor's verbatim error; (2) the call still
//!    propagates `Err(AgentError::LaneBFail(_))` to the caller (the
//!    existing line-817 `lane_b` HardFail also fires — that is not
//!    under test, but is fine to coexist).

use std::collections::HashSet;
use std::sync::Arc;

use atlas_agents::events::{AgentEvent, EventBus};
use atlas_agents::runtime::audit::Stage;
use atlas_agents::runtime::{AgentRequest, ContentSha, ForProviderFn};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{default_tool_catalog, AgentRuntime, Semaphores};
use atlas_engine::llm_cache::LlmResponseCache;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, Provider};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::TryRecvError;

/// Backend that returns `LlmError::Invocation(label)` on every call.
struct AlwaysErroringBackend {
    label: String,
    error_text: String,
}

impl AlwaysErroringBackend {
    fn arc(label: impl Into<String>, error_text: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            label: label.into(),
            error_text: error_text.into(),
        })
    }
}

#[async_trait::async_trait]
impl LlmBackend for AlwaysErroringBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation(self.error_text.clone()))
    }
    async fn call_async(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation(self.error_text.clone()))
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

/// Backend that always returns the same canned response.
struct AlwaysSucceedingBackend {
    label: String,
    response: Value,
}

impl AlwaysSucceedingBackend {
    fn arc(label: impl Into<String>, response: Value) -> Arc<Self> {
        Arc::new(Self {
            label: label.into(),
            response,
        })
    }
}

#[async_trait::async_trait]
impl LlmBackend for AlwaysSucceedingBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation("sync path unused in this test".into()))
    }
    async fn call_async(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
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

/// Producer response that claims `strong` but emits no
/// `evidence_pointers` → Lane A evidence-floor clamps to Declines →
/// Lane B fires. Mirrors the helper used by `cross_provider_audit_routing.rs`.
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

/// Drain a subscriber non-blockingly, ignoring Lagged. Stops on Empty
/// or Closed.
fn drain_events(rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => return out,
            Err(TryRecvError::Lagged(_)) => continue,
        }
    }
}

#[tokio::test]
async fn producer_backend_error_emits_hardfail_then_propagates() {
    let audit_tmp = tempfile::tempdir().expect("tempdir");

    let producer = AlwaysErroringBackend::arc(
        "erroring-producer",
        "synthetic upstream rejection from producer",
    );
    let runtime = build_runtime(
        producer.clone() as Arc<dyn LlmBackend>,
        None,
        TransportFlavour::HttpAnthropic,
        audit_tmp.path().to_path_buf(),
    );

    let mut rx = runtime.event_bus.subscribe();
    let result = runtime
        .call_agent(classify_request("foo", TransportFlavour::HttpAnthropic))
        .await;

    let err = result.expect_err("producer-fail must propagate Err to the caller");
    let err_summary = err.to_string();
    assert!(
        err_summary.contains("synthetic upstream rejection from producer"),
        "propagated Err should carry the backend's verbatim error text: {err_summary}"
    );

    let events = drain_events(&mut rx);
    let hardfails: Vec<_> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::HardFail {
                error_kind,
                error_summary,
                ..
            } => Some((error_kind.clone(), error_summary.clone())),
            _ => None,
        })
        .collect();
    let backend_kind = hardfails
        .iter()
        .find(|(kind, _)| kind == "backend")
        .unwrap_or_else(|| {
            panic!(
                "expected HardFail event with error_kind=\"backend\" on the bus; observed: {hardfails:?}"
            )
        });
    assert!(
        backend_kind
            .1
            .contains("synthetic upstream rejection from producer"),
        "HardFail.error_summary should carry the backend's verbatim error text: {}",
        backend_kind.1
    );
}

#[tokio::test]
async fn auditor_backend_error_emits_audit_hardfail_then_propagates() {
    let audit_tmp = tempfile::tempdir().expect("tempdir");

    let producer = AlwaysSucceedingBackend::arc("ok-producer", weak_classify_response());
    let auditor = AlwaysErroringBackend::arc(
        "erroring-auditor",
        "synthetic upstream rejection from auditor",
    );

    let auditor_for_closure = auditor.clone();
    // Producer transport is HttpAnthropic → Lane B audit looks up the
    // cross-provider backend via `Provider::OpenAi`.
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

    let err = result.expect_err("auditor-fail must propagate Err via LaneBFail");
    let err_summary = err.to_string();
    assert!(
        err_summary.contains("synthetic upstream rejection from auditor"),
        "propagated Err should carry the auditor backend's verbatim error: {err_summary}"
    );

    let events = drain_events(&mut rx);
    let hardfails: Vec<_> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::HardFail {
                error_kind,
                error_summary,
                ..
            } => Some((error_kind.clone(), error_summary.clone())),
            _ => None,
        })
        .collect();
    let audit_kind = hardfails
        .iter()
        .find(|(kind, _)| kind == "audit_backend")
        .unwrap_or_else(|| {
            panic!(
                "expected HardFail event with error_kind=\"audit_backend\" on the bus; observed: {hardfails:?}"
            )
        });
    assert!(
        audit_kind
            .1
            .contains("synthetic upstream rejection from auditor"),
        "HardFail.error_summary should carry the auditor backend's verbatim error: {}",
        audit_kind.1
    );
}
