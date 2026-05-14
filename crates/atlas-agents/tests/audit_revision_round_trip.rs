//! PR-4: revision-prompt round-trip integration test (plan §4 Task 4.6,
//! brainstorm §7.3). Verifies:
//!
//! 1. `request_revision` → producer is re-invoked with the auditor's
//!    reason embedded as a system-prompt addendum (the `AUDITOR'S
//!    CRITIQUE` marker substring + the reason text).
//! 2. Cumulative-budget rule: once `lane_b_revisions` increments to 1,
//!    a subsequent `request_revision` escalates to `HardFail` per
//!    `resolve_audit_verdict`.
//!
//! Cross-provider routing: the test uses producer (Anthropic) +
//! auditor (OpenAI) backends + a `for_provider` closure returning the
//! auditor for `Provider::OpenAi`. This exercises the same auditor
//! closure that production runs would invoke.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use atlas_agents::events::EventBus;
use atlas_agents::runtime::audit::Stage;
use atlas_agents::runtime::{AgentRequest, ContentSha, ForProviderFn};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{default_tool_catalog, AgentError, AgentRuntime, Semaphores};
use atlas_engine::llm_cache::LlmResponseCache;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, Provider};
use serde_json::{json, Value};

/// Boxed responder closure: `(call_index, conversation) -> Value`.
type Responder = Box<dyn FnMut(usize, &str) -> Value + Send>;

/// A capture-everything async backend. Records every (conversation,
/// system_prompt) pair it's called with and lets the test inject a
/// callback that returns the response.
struct CaptureBackend {
    label: String,
    /// Convoy of every conversation string the backend saw.
    seen_conversations: Mutex<Vec<String>>,
    /// Callback that maps `call_index -> Value`. The test prepares the
    /// canned responses + the callback returns by index.
    responder: Mutex<Responder>,
    call_count: std::sync::atomic::AtomicU32,
}

impl CaptureBackend {
    fn new(label: impl Into<String>, responder: Responder) -> Arc<Self> {
        Arc::new(Self {
            label: label.into(),
            seen_conversations: Mutex::new(Vec::new()),
            responder: Mutex::new(responder),
            call_count: std::sync::atomic::AtomicU32::new(0),
        })
    }

    fn call_count(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn conversations(&self) -> Vec<String> {
        self.seen_conversations.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LlmBackend for CaptureBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation("CaptureBackend is async-only".into()))
    }
    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst) as usize;
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
        self.seen_conversations
            .lock()
            .unwrap()
            .push(conversation.clone());
        let mut responder = self.responder.lock().unwrap();
        Ok((*responder)(idx, &conversation))
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

/// Producer's response: classify envelope claiming `confidence_grade:
/// strong` but with NO `evidence_pointers`, so Lane A's evidence-floor
/// clamps the grade to `Declines` → Lane B fires.
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

/// Producer's response after seeing the revision addendum: still no
/// `evidence_pointers`, but explicitly acknowledges the auditor's
/// reason in the rationale so the test can lock in that the producer
/// received the addendum.
fn revised_classify_response_acknowledging(reason_substring: &str) -> Value {
    text_block(format!(
        concat!(
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
            "rationale: \"acknowledging auditor: {reason}\"\n",
            "```\n"
        ),
        reason = reason_substring
    ))
}

fn audit_response_request_revision(reason: &str) -> Value {
    text_block(format!(
        concat!(
            "```yaml\n",
            "verdict: request_revision\n",
            "reason: |\n",
            "  {reason}\n",
            "```\n"
        ),
        reason = reason
    ))
}

fn audit_response_accept() -> Value {
    text_block(concat!(
        "```yaml\n",
        "verdict: accept\n",
        "reason: |\n",
        "  revised output addresses the prior critique.\n",
        "```\n"
    ))
}

fn build_runtime(
    producer: Arc<CaptureBackend>,
    auditor: Arc<CaptureBackend>,
    audit_dir: std::path::PathBuf,
) -> AgentRuntime {
    let auditor_clone = auditor.clone();
    let for_provider: Arc<ForProviderFn> = Arc::new(move |p: Provider| {
        if p == Provider::OpenAi {
            Some(auditor_clone.clone() as Arc<dyn LlmBackend>)
        } else {
            None
        }
    });
    AgentRuntime {
        backend_router: producer as Arc<dyn LlmBackend>,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: Arc::new(EventBus::new(1024)),
        semaphores: Semaphores::defaults(),
        // HttpAnthropic so the producer's provider is Anthropic; the
        // auditor's `cross()` is OpenAi → routed via `for_provider`.
        default_transport: TransportFlavour::HttpAnthropic,
        default_max_steps: 4,
        max_iterations: 1,
        for_provider: Some(for_provider),
        mcp_server: None,
        audit_dir,
    }
}

fn classify_request(target_id: impl Into<String>) -> AgentRequest {
    let target_id = target_id.into();
    let candidate_ids: HashSet<String> = std::iter::once(target_id.clone()).collect();
    AgentRequest {
        stage: Stage::Classify,
        target_id: target_id.clone(),
        iteration: 1,
        transport: TransportFlavour::HttpAnthropic,
        // Non-empty so the prompt is plausible; Lane A doesn't
        // inspect this for classify-stage validity beyond candidate-id
        // membership (target_id is in the candidate set, so it
        // passes).
        initial_prompt: format!(
            "You are Atlas's classify agent. Classify component `{target_id}`.\n"
        ),
        fingerprint_inputs: Vec::new(),
        candidate_ids,
        prior_model_sha: Some(ContentSha("0".repeat(64))),
        lane_b_revisions: 0,
    }
}

#[tokio::test]
async fn auditor_request_revision_threads_reason_into_producer_retry() {
    let audit_tmp = tempfile::tempdir().unwrap();

    // Producer: call 0 returns weak (Lane B will fire); call 1
    // returns the revision-acknowledging response.
    let revision_reason = "needs more evidence pointers";
    let producer_reason = revision_reason.to_string();
    let producer = CaptureBackend::new(
        "anthropic-producer",
        Box::new(move |idx, _conv| match idx {
            0 => weak_classify_response(),
            _ => revised_classify_response_acknowledging(&producer_reason),
        }),
    );

    // Auditor: call 0 returns request_revision; call 1 returns accept
    // (so the revised output passes audit).
    let auditor_reason = revision_reason.to_string();
    let auditor = CaptureBackend::new(
        "openai-auditor",
        Box::new(move |idx, _conv| match idx {
            0 => audit_response_request_revision(&auditor_reason),
            _ => audit_response_accept(),
        }),
    );

    let runtime = build_runtime(
        producer.clone(),
        auditor.clone(),
        audit_tmp.path().to_path_buf(),
    );
    let result = runtime.call_agent(classify_request("foo")).await;
    assert!(
        result.is_ok(),
        "revised output should pass audit on second pass; got {result:?}"
    );

    // Producer called twice: original + revision.
    assert_eq!(
        producer.call_count(),
        2,
        "producer should be called twice (initial + revision)"
    );

    // Auditor called twice: once for the initial output, once for the
    // revised output (which the canned response accepts).
    assert_eq!(
        auditor.call_count(),
        2,
        "auditor should be called twice (initial + revised-output audit)"
    );

    // The second producer call's conversation must contain the
    // revision addendum markers + the auditor's reason verbatim.
    let conversations = producer.conversations();
    let second = &conversations[1];
    assert!(
        second.contains("AUDITOR'S CRITIQUE"),
        "second producer call must carry the AUDITOR'S CRITIQUE marker; \
         got conversation:\n{second}"
    );
    assert!(
        second.contains(revision_reason),
        "second producer call must carry the auditor's reason verbatim; \
         got conversation:\n{second}"
    );
    assert!(
        second.contains("PRIOR ATTEMPT"),
        "second producer call must carry the PRIOR ATTEMPT marker"
    );
}

#[tokio::test]
async fn cumulative_retry_budget_escalates_to_hard_fail() {
    let audit_tmp = tempfile::tempdir().unwrap();

    // Producer returns a slightly-different weak response on each
    // call so the producer output_sha varies — busting the on-disk
    // verdict cache so each audit call fires fresh against the
    // auditor backend. Both responses are evidence-empty so Lane A
    // still clamps the grade to Declines on both passes.
    let producer = CaptureBackend::new(
        "anthropic-producer",
        Box::new(move |idx, _conv| {
            // The `rationale` field varies per call to change the sha
            // without changing semantically-load-bearing fields.
            text_block(format!(
                concat!(
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
                    "rationale: \"call {idx}: still no evidence\"\n",
                    "```\n"
                ),
                idx = idx
            ))
        }),
    );

    // Auditor always returns request_revision — the second invocation
    // (against the revised producer's still-weak output) MUST escalate
    // to HardFail because the cumulative budget is exhausted
    // (lane_a_retries=0 + lane_b_revisions=1 = 1 ≥ cap).
    let auditor = CaptureBackend::new(
        "openai-auditor",
        Box::new(move |_idx, _conv| audit_response_request_revision("still no evidence")),
    );

    let runtime = build_runtime(
        producer.clone(),
        auditor.clone(),
        audit_tmp.path().to_path_buf(),
    );
    let result = runtime.call_agent(classify_request("foo")).await;

    match result {
        Err(AgentError::LaneBFail(msg)) => {
            assert!(
                msg.contains("budget exhausted") || msg.contains("retry budget"),
                "LaneBFail message must explain the cumulative-budget rule; \
                 got: {msg}"
            );
        }
        other => panic!("expected LaneBFail after cumulative-budget exhaustion; got {other:?}"),
    }

    // Producer called twice: initial + one revision (the second
    // revision would never fire because the cumulative budget
    // escalates first).
    assert_eq!(
        producer.call_count(),
        2,
        "producer called twice before cumulative budget escalates"
    );
    // Auditor called twice: once for initial, once for revision (the
    // escalation happens AFTER the second audit returns
    // request_revision).
    assert_eq!(auditor.call_count(), 2);
}

#[tokio::test]
async fn audit_dir_receives_persisted_verdict_after_successful_audit() {
    // Smoke check that the audit closure persists the verdict to
    // `<audit_dir>/<stage>/<target_id>.yaml` after a fresh audit.
    let audit_tmp = tempfile::tempdir().unwrap();
    let audit_dir = audit_tmp.path().to_path_buf();

    let producer = CaptureBackend::new(
        "anthropic-producer",
        Box::new(move |_idx, _conv| weak_classify_response()),
    );
    let auditor = CaptureBackend::new(
        "openai-auditor",
        Box::new(move |_idx, _conv| audit_response_accept()),
    );

    let runtime = build_runtime(producer, auditor, audit_dir.clone());
    let _ = runtime.call_agent(classify_request("foo")).await.unwrap();

    let verdict_path = audit_dir.join("classify").join("foo.yaml");
    let transcript_path = audit_dir.join("classify").join("foo.audit-transcript");
    assert!(
        verdict_path.exists(),
        "verdict YAML must land at {verdict_path:?}"
    );
    assert!(
        transcript_path.exists(),
        "audit transcript sibling must land at {transcript_path:?}"
    );

    let yaml = std::fs::read_to_string(&verdict_path).unwrap();
    assert!(yaml.contains("verdict: accept"));
    assert!(yaml.contains("provider: anthropic"));
    assert!(yaml.contains("provider: openai"));
}
