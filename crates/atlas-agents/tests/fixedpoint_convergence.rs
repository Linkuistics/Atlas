//! PR-5 fixed-point convergence acceptance tests
//! (plan §4 Task 5.7, recast §4.4).
//!
//! Validates:
//!
//! 1. Convergence detection: a workspace whose iteration is idempotent
//!    (every iteration produces the same `L9Projection`) returns after
//!    the second iteration confirms the sha is unchanged.
//! 2. Hard fail on divergence: a workspace whose iterations never
//!    stabilise hits the `max_iter` cap and surfaces
//!    `AgentError::FixedpointDiverged`.
//! 3. Per-iteration cache key: the `iteration_number` field of the
//!    transcript-cache fingerprint changes across iterations, so two
//!    iterations produce distinct cache keys.

use std::sync::Arc;

use atlas_agents::events::{AgentEvent, EventBus};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{
    default_tool_catalog, AgentError, AgentRuntime, Semaphores, Workspace as AgentsWorkspace,
};
use atlas_engine::llm_cache::{AgentInputFingerprint, LlmResponseCache};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};
use serde_json::{json, Value};

/// Mode toggle for the convergence backend.
#[derive(Clone, Copy)]
enum DivergenceMode {
    /// Every call returns the same canned response — the projection
    /// is idempotent across iterations.
    Idempotent,
    /// Every call returns a response keyed on the iteration number,
    /// so each iteration produces a different projection.
    PerIterationDistinct,
}

/// Substring-keyed async backend that decorates its response payload
/// with the iteration number when `PerIterationDistinct`. Lets the
/// test drive both convergence and divergence from one fixture.
struct ConvergenceBackend {
    fingerprint: LlmFingerprint,
    mode: DivergenceMode,
    call_count: std::sync::atomic::AtomicU32,
}

impl ConvergenceBackend {
    fn new(mode: DivergenceMode) -> Self {
        Self {
            fingerprint: LlmFingerprint {
                template_sha: [0u8; 32],
                ontology_sha: [0u8; 32],
                model_id: "convergence-test-backend".to_string(),
                backend_version: "0".to_string(),
            },
            mode,
            call_count: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

#[async_trait::async_trait]
impl LlmBackend for ConvergenceBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation("async-only".into()))
    }
    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let n = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // WI-1: agent-runtime requests carry the prompt under
        // `rendered_prompt`.
        let conversation: &str = req
            .rendered_prompt
            .as_deref()
            .or_else(|| req.inputs.get("conversation").and_then(Value::as_str))
            .unwrap_or("");
        // PR-4: audit prompts open with "You are an auditor for an
        // Atlas agent's output". Return a fenced YAML accept verdict
        // so Lane B can complete on the synthetic workspace.
        if conversation.contains("You are an auditor") {
            return Ok(json!({
                "content": [{
                    "type": "text",
                    "text": "```yaml\nverdict: accept\nreason: |\n  Synthetic-test producer; transcript is empty.\n```"
                }]
            }));
        }
        // Wrap the response in a text-block envelope so the
        // tool_loop_http parser extracts the JSON payload.
        let inner = match self.mode {
            DivergenceMode::Idempotent => json!({ "components": [{ "id": "foo" }] }),
            DivergenceMode::PerIterationDistinct => {
                json!({ "components": [{ "id": "foo" }], "_call": n })
            }
        };
        Ok(json!({
            "content": [
                { "type": "text", "text": inner.to_string() }
            ]
        }))
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

#[tokio::test]
async fn fixedpoint_converges_on_idempotent_workspace_after_two_iterations() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_overrides(root);

    let backend: Arc<dyn LlmBackend> =
        Arc::new(ConvergenceBackend::new(DivergenceMode::Idempotent));
    let bus = Arc::new(EventBus::new(1024));
    let mut rx = bus.subscribe();

    let runtime = AgentRuntime {
        backend_router: backend,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: bus.clone(),
        semaphores: Semaphores::defaults(),
        default_transport: TransportFlavour::HttpAnthropic,
        default_max_steps: 4,
        max_iterations: 5,
        for_provider: None,
        mcp_server: None,
        audit_dir: dir.path().join("audit"),
    };
    let workspace = AgentsWorkspace::new(root);
    let projection = runtime
        .run_workspace(&workspace)
        .await
        .expect("idempotent workspace must converge");
    assert!(projection.components.contains_key("foo"));

    // Collect events. Drain until we hit RuntimeComplete.
    let mut events = Vec::new();
    while let Ok(ev) = rx.recv().await {
        let done = matches!(ev, AgentEvent::RuntimeComplete);
        events.push(ev);
        if done {
            break;
        }
    }

    let iter_boundaries: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::IterationBoundary { iter, .. } => Some(*iter),
            _ => None,
        })
        .collect();
    assert_eq!(
        iter_boundaries,
        vec![1, 2],
        "idempotent workspace must take exactly two iterations (iter 1 baseline, iter 2 confirms convergence)"
    );
}

#[tokio::test]
async fn fixedpoint_hard_fails_when_max_iter_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_overrides(root);

    let backend: Arc<dyn LlmBackend> = Arc::new(ConvergenceBackend::new(
        DivergenceMode::PerIterationDistinct,
    ));
    let bus = Arc::new(EventBus::new(1024));

    let runtime = AgentRuntime {
        backend_router: backend,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: bus.clone(),
        semaphores: Semaphores::defaults(),
        default_transport: TransportFlavour::HttpAnthropic,
        default_max_steps: 4,
        // Set a tight cap so the test runs quickly. Per-iteration
        // distinct projections cannot converge under any cap.
        max_iterations: 3,
        for_provider: None,
        mcp_server: None,
        audit_dir: dir.path().join("audit"),
    };
    let workspace = AgentsWorkspace::new(root);
    let err = runtime
        .run_workspace(&workspace)
        .await
        .expect_err("divergent workspace must hard-fail");
    match err {
        AgentError::FixedpointDiverged {
            iterations,
            last_changed_agents: _,
        } => {
            assert_eq!(iterations, 3);
        }
        other => panic!("expected FixedpointDiverged, got {other:?}"),
    }
}

#[test]
fn fixedpoint_caches_per_iteration_via_iteration_number_in_fingerprint() {
    // Two AgentInputFingerprints differing only in `iteration_number`
    // must produce distinct cache keys. The fixed-point loop relies on
    // this so iteration 2's transcript-cache hits don't shadow
    // iteration 1's (and vice versa).
    let base = AgentInputFingerprint {
        stage_id: "classify".to_string(),
        agent_id: "classify::foo#i1".to_string(),
        agent_version: "v0".to_string(),
        prompt_template_sha: [1u8; 32],
        tool_catalog_sha: [2u8; 32],
        model_id: "test".to_string(),
        backend_version: "v0".to_string(),
        transport_flavour: "http_anthropic".to_string(),
        target_input_shas: vec![],
        iteration_number: 1,
        prior_model_sha: None,
        override_content_sha: None,
    };
    let mut iter2 = base.clone();
    iter2.iteration_number = 2;
    assert_ne!(
        base.to_cache_key(),
        iter2.to_cache_key(),
        "iteration_number must contribute to the transcript-cache key"
    );
}
