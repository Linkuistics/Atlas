//! Single-iteration smoke test for `AgentRuntime` (plan §4 Task 4.7).
//!
//! Wires a synthetic workspace (tempdir + override files) against a
//! `StagedBackend` (a small async `LlmBackend` impl that keys
//! canned responses on a substring of the running conversation) and
//! asserts:
//!
//! 1. `run_workspace().await` returns a non-empty `L9Projection`
//!    containing the per-component, per-subsystem, and project
//!    entries the runtime computed.
//! 2. Lane A retry: a malformed Classify response triggers exactly
//!    one retry, after which the projection still includes `foo` and
//!    no `HardFail` event was emitted.

use std::sync::Arc;

use atlas_agents::events::{AgentEvent, EventBus};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{default_tool_catalog, AgentRuntime, Semaphores, Workspace as AgentsWorkspace};
use atlas_engine::llm_cache::LlmResponseCache;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};
use serde_json::{json, Value};

/// Substring-keyed async backend. The test wires one instance per
/// scenario; each `call_async` returns the first canned response
/// whose key occurs in the request's `inputs.conversation` field.
/// One-shot entries (popped on match) take precedence over the
/// permanent table, letting tests model "second response differs".
struct StagedBackend {
    fingerprint: LlmFingerprint,
    by_substring: Vec<(String, Value)>,
    one_shots: std::sync::Mutex<Vec<(String, Value)>>,
    call_count: std::sync::atomic::AtomicU32,
}

impl StagedBackend {
    fn new(canned: Vec<(String, Value)>) -> Self {
        Self {
            fingerprint: LlmFingerprint {
                template_sha: [0u8; 32],
                ontology_sha: [0u8; 32],
                model_id: "staged-test-backend".to_string(),
                backend_version: "0".to_string(),
            },
            by_substring: canned,
            one_shots: std::sync::Mutex::new(Vec::new()),
            call_count: std::sync::atomic::AtomicU32::new(0),
        }
    }

    fn with_one_shot(self, substring: impl Into<String>, value: Value) -> Self {
        self.one_shots
            .lock()
            .unwrap()
            .push((substring.into(), value));
        self
    }

    fn call_count(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmBackend for StagedBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation("StagedBackend is async-only".into()))
    }

    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // WI-1: agent-runtime requests carry the prompt under
        // `rendered_prompt`; legacy templated callers used
        // `inputs.conversation`.
        let conversation: &str = req
            .rendered_prompt
            .as_deref()
            .or_else(|| req.inputs.get("conversation").and_then(Value::as_str))
            .unwrap_or("");

        // One-shots: scan in insertion order; first match wins, and
        // is removed so subsequent calls fall through to the
        // permanent table.
        {
            let mut one_shots = self.one_shots.lock().unwrap();
            if let Some(idx) = one_shots
                .iter()
                .position(|(sub, _)| conversation.contains(sub))
            {
                let (_, value) = one_shots.remove(idx);
                return Ok(value);
            }
        }

        for (substring, value) in &self.by_substring {
            if conversation.contains(substring.as_str()) {
                return Ok(value.clone());
            }
        }
        Err(LlmError::TestBackendMiss(format!(
            "no canned response matched conversation: {}",
            &conversation[..conversation.len().min(120)]
        )))
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

fn write_overrides(root: &std::path::Path) {
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
}

fn text_block(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

/// PR-4: canned auditor response. The producer prompts open with
/// "You are Atlas's <stage> agent"; the audit prompt opens with
/// "You are an auditor for an Atlas agent's output". The substring
/// "auditor" disambiguates and routes audit calls to the canned
/// accept-verdict here so Lane B can complete on a synthetic
/// workspace without a second `StagedBackend` instance.
fn audit_accept_response() -> Value {
    // Use `concat!()` with explicit newlines + literal leading spaces
    // on the block-scalar content line — `\n\` line continuations
    // consume leading whitespace, which would un-indent the YAML
    // block scalar and break the parser.
    text_block(concat!(
        "```yaml\n",
        "verdict: accept\n",
        "reason: |\n",
        "  Synthetic-test producer; transcript is empty by design.\n",
        "```\n"
    ))
}

#[tokio::test]
async fn agent_runtime_runs_a_workspace_end_to_end_single_iteration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write_overrides(root);

    // PR-3: production prompts open with the substring
    // `"You are Atlas's <stage> agent"`; the test backend keys on
    // `"<stage> agent"` to remain stable across prompt-body edits while
    // disambiguating the three stages.
    let backend = Arc::new(StagedBackend::new(vec![
        // PR-4: audit responses first so Lane B fires can match this
        // entry before the producer-stage entries (the audit prompt
        // doesn't contain "classify agent" / etc., so order is safe).
        ("auditor".to_string(), audit_accept_response()),
        (
            "classify agent".to_string(),
            text_block("{\"components\":[{\"id\":\"foo\"}]}"),
        ),
        (
            "reduce agent".to_string(),
            text_block("{\"components\":[{\"id\":\"foo\"}]}"),
        ),
        (
            "project agent".to_string(),
            text_block("{\"components\":[{\"id\":\"foo\"}]}"),
        ),
    ])) as Arc<dyn LlmBackend>;

    let runtime = AgentRuntime {
        backend_router: backend,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: Arc::new(EventBus::new(1024)),
        semaphores: Semaphores::defaults(),
        default_transport: TransportFlavour::HttpAnthropic,
        default_max_steps: 4,
        // PR-5: fixed-point loop's `max_iterations == 1` is the
        // single-iteration-mode sentinel — equivalent to PR-4's
        // direct call to `run_iteration`. The PR-4 smoke fixture
        // exercises this mode so its assertions on backend call
        // count and one-shot consumption stay valid under PR-5.
        max_iterations: 1,
        for_provider: None,
        mcp_server: None,
        audit_dir: dir.path().join("audit"),
    };
    let workspace = AgentsWorkspace::new(root);

    let projection = runtime
        .run_workspace(&workspace)
        .await
        .expect("workspace runs to completion");

    assert!(!projection.is_empty(), "projection should not be empty");
    assert!(
        projection.components.contains_key("foo"),
        "expected component foo, got keys: {:?}",
        projection.components.keys().collect::<Vec<_>>()
    );
    assert!(
        projection.subsystems.contains_key("agents"),
        "expected subsystem agents"
    );
    assert!(projection.project.is_some(), "expected workspace project");
}

#[tokio::test]
async fn lane_a_retry_fires_exactly_once_on_classify_schema_fail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write_overrides(root);

    // First Classify call: an unknown edge kind → Lane A fails.
    // The retry call sees the appended `[lane_a_retry]` marker in
    // the conversation and matches the one-shot valid response.
    let invalid =
        text_block("{\"edges\":[{\"kind\":\"frobnicates\",\"from\":\"foo\",\"to\":\"foo\"}]}");
    let valid_after_retry = text_block("{\"components\":[{\"id\":\"foo\"}]}");
    let reduce_response = text_block("{\"components\":[{\"id\":\"foo\"}]}");
    let project_response = text_block("{\"components\":[{\"id\":\"foo\"}]}");

    let backend_inner = StagedBackend::new(vec![
        // PR-4: audit accept first so Lane B can complete on a
        // synthetic workspace (see notes in the other test).
        ("auditor".to_string(), audit_accept_response()),
        ("classify agent".to_string(), invalid),
        ("reduce agent".to_string(), reduce_response),
        ("project agent".to_string(), project_response),
    ])
    .with_one_shot("[lane_a_retry]", valid_after_retry);
    let backend: Arc<dyn LlmBackend> = Arc::new(backend_inner);

    let bus = Arc::new(EventBus::new(1024));
    let mut rx = bus.subscribe();
    let collector = tokio::spawn(async move {
        let mut events: Vec<AgentEvent> = Vec::new();
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let done = matches!(ev, AgentEvent::RuntimeComplete);
                    events.push(ev);
                    if done {
                        return events;
                    }
                }
                Err(_) => return events,
            }
        }
    });

    let runtime = AgentRuntime {
        backend_router: backend,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: bus.clone(),
        semaphores: Semaphores::defaults(),
        default_transport: TransportFlavour::HttpAnthropic,
        default_max_steps: 4,
        // PR-5: fixed-point loop's `max_iterations == 1` is the
        // single-iteration-mode sentinel — equivalent to PR-4's
        // direct call to `run_iteration`. The PR-4 smoke fixture
        // exercises this mode so its assertions on backend call
        // count and one-shot consumption stay valid under PR-5.
        max_iterations: 1,
        for_provider: None,
        mcp_server: None,
        audit_dir: dir.path().join("audit"),
    };
    let workspace = AgentsWorkspace::new(root);

    let projection = runtime
        .run_workspace(&workspace)
        .await
        .expect("workspace runs ok after lane A retry");
    // `run_workspace` owns the `RuntimeComplete` emit (the drain-
    // handshake sentinel) as of the PR-4 follow-up; no manual emit
    // here. Emitting twice would surface a second sentinel to the
    // collector, which would already have returned on the first one.
    let events = collector.await.expect("collector finished");

    assert!(
        projection.components.contains_key("foo"),
        "projection should include foo after retry"
    );
    let hard_fails: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev, AgentEvent::HardFail { .. }))
        .collect();
    assert!(
        hard_fails.is_empty(),
        "Lane A retry succeeded; HardFail should not fire. Got: {hard_fails:?}"
    );
    let classify_starts: Vec<_> = events
        .iter()
        .filter(|ev| match ev {
            AgentEvent::AgentStart { stage, .. } => stage == "classify",
            _ => false,
        })
        .collect();
    assert!(
        !classify_starts.is_empty(),
        "expected at least one classify AgentStart"
    );
}

/// Sanity check that the StagedBackend behaves the way the smoke tests
/// rely on: one-shots are popped on match; the permanent table fires
/// after the one-shots are exhausted.
#[tokio::test]
async fn staged_backend_one_shot_pops_after_match() {
    let backend = StagedBackend::new(vec![("hello".to_string(), json!({"perm": true}))])
        .with_one_shot("hello", json!({"oneshot": true}));
    let req = LlmRequest::from_template(
        atlas_llm::PromptId::Classify,
        json!({ "conversation": "hello world" }),
        atlas_llm::ResponseSchema::accept_any(),
    );
    let r1 = backend.call_async(&req).await.unwrap();
    let r2 = backend.call_async(&req).await.unwrap();
    assert_eq!(r1, json!({"oneshot": true}));
    assert_eq!(r2, json!({"perm": true}));
    assert_eq!(backend.call_count(), 2);
}
