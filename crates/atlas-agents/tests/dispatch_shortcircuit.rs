//! PR-5 dispatch-shortcircuit acceptance tests (plan §4 Task 5.5).
//!
//! Verifies the override-file shortcircuit AND the LLM-decided
//! dispatch path in `dispatch_subsystems` / `dispatch_components`:
//!
//! - With an override present: `CacheHit { source:
//!   DispatchedFromOverride }` fires; the parsed partitions match the
//!   override file; the LLM dispatch agent does NOT fire.
//! - Without an override: the LLM dispatch agent fires via
//!   `runtime.call_agent`; the returned partitions match the backend's
//!   canned response; no `DispatchedFromOverride` cache hit is emitted
//!   (the path went through `call_agent`, not the synthetic
//!   shortcircuit).
//! - With an invalid override: Lane A surfaces `OverrideRequired` (PR-5
//!   treats override parse + structural failures as `OverrideRequired`
//!   rather than `LaneAFail` so users see a clearer authoring error;
//!   the recast spec's "Lane A fail" framing covers both).
//! - Cache-invariant rule (recast §6.1): adding an override invalidates
//!   the LLM-dispatch transcript; removing an override invalidates the
//!   synthetic-from-override transcript. Both directions are checked
//!   by asserting fingerprint inequality across the relevant
//!   transition.

use std::sync::Arc;

use atlas_agents::events::{AgentEvent, CacheHitSource, EventBus};
use atlas_agents::runtime::dispatch::{
    dispatch_components, dispatch_fingerprint, dispatch_subsystems, SubsystemPartition,
    COMPONENTS_OVERRIDE_FILENAME, SUBSYSTEMS_OVERRIDE_FILENAME,
};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{
    default_tool_catalog, AgentError, AgentRuntime, Semaphores, Stage as AgentStage,
    Workspace as AgentsWorkspace,
};
use atlas_engine::llm_cache::LlmResponseCache;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};
use serde_json::{json, Value};

/// Test stub backend. The dispatch shortcircuit path never invokes
/// the backend — those tests assert the backend's call counter stays
/// at zero. Constructing a real `StagedBackend` would be misleading
/// since the override-present path never reaches it.
struct CountingBackend {
    call_count: std::sync::atomic::AtomicU32,
}

impl CountingBackend {
    fn new() -> Self {
        Self {
            call_count: std::sync::atomic::AtomicU32::new(0),
        }
    }
    fn calls(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmBackend for CountingBackend {
    fn call(&self, _req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(LlmError::Invocation(
            "unused in dispatch shortcircuit tests".into(),
        ))
    }
    async fn call_async(&self, _req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(LlmError::Invocation(
            "unused in dispatch shortcircuit tests".into(),
        ))
    }
    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: "dispatch-shortcircuit-test".to_string(),
            backend_version: "v0".to_string(),
        }
    }
}

/// Substring-keyed async backend for the LLM-dispatch firing test.
/// Mirrors the `StagedBackend` shape used by the PR-4 single-iteration
/// smoke, scoped down to what `tests/dispatch_shortcircuit.rs` needs.
struct DispatchStagedBackend {
    by_substring: Vec<(String, Value)>,
    call_count: std::sync::atomic::AtomicU32,
}

impl DispatchStagedBackend {
    fn new(canned: Vec<(String, Value)>) -> Self {
        Self {
            by_substring: canned,
            call_count: std::sync::atomic::AtomicU32::new(0),
        }
    }
    fn calls(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmBackend for DispatchStagedBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation(
            "DispatchStagedBackend is async-only".into(),
        ))
    }
    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let conversation = req
            .inputs
            .get("conversation")
            .and_then(Value::as_str)
            .unwrap_or("");
        for (substring, value) in &self.by_substring {
            if conversation.contains(substring.as_str()) {
                return Ok(value.clone());
            }
        }
        Err(LlmError::TestBackendMiss(format!(
            "no canned response matched dispatch conversation: {}",
            &conversation[..conversation.len().min(160)]
        )))
    }
    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: "dispatch-staged-test".to_string(),
            backend_version: "v0".to_string(),
        }
    }
}

fn make_runtime(backend: Arc<dyn LlmBackend>) -> AgentRuntime {
    AgentRuntime {
        backend_router: backend,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: Arc::new(EventBus::new(64)),
        semaphores: Semaphores::defaults(),
        default_transport: TransportFlavour::HttpAnthropic,
        default_max_steps: 4,
        max_iterations: 1,
        for_provider: None,
    }
}

fn write_minimal_overrides(root: &std::path::Path) {
    std::fs::write(
        root.join(SUBSYSTEMS_OVERRIDE_FILENAME),
        "schema_version: 1\nsubsystems:\n  - id: agents\n    members: [foo]\n",
    )
    .unwrap();
    std::fs::write(
        root.join(COMPONENTS_OVERRIDE_FILENAME),
        "schema_version: 1\ncomponents:\n  foo:\n    subsystem: agents\n",
    )
    .unwrap();
}

#[tokio::test]
async fn dispatch_with_override_file_emits_synthetic_cache_hit() {
    let dir = tempfile::tempdir().unwrap();
    write_minimal_overrides(dir.path());

    let backend = Arc::new(CountingBackend::new());
    let runtime = make_runtime(backend.clone() as Arc<dyn LlmBackend>);
    let workspace = AgentsWorkspace::new(dir.path());

    let mut rx = runtime.event_bus.subscribe();

    let partitions = dispatch_subsystems(&runtime, &workspace).await.unwrap();
    assert_eq!(partitions.len(), 1);
    assert_eq!(partitions[0].id, "agents");

    // Drain the bus and verify exactly one CacheHit with
    // DispatchedFromOverride landed.
    let mut hits = 0u32;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::CacheHit { source, .. } = ev {
            if matches!(source, CacheHitSource::DispatchedFromOverride) {
                hits += 1;
            }
        }
    }
    assert_eq!(
        hits, 1,
        "exactly one DispatchedFromOverride CacheHit must fire per dispatch_subsystems call"
    );
    assert_eq!(
        backend.calls(),
        0,
        "the shortcircuit path must NOT invoke the LLM backend"
    );
}

#[tokio::test]
async fn dispatch_without_override_file_fires_llm_agent() {
    // PR-5 follow-up (plan §4 Task 5.5): when no override file is
    // present, the dispatcher routes through `runtime.call_agent`
    // (Lane A + cache + per-agent fingerprint). This test asserts:
    // (a) the backend was called at least once with a dispatch
    //     stage-matching request,
    // (b) the returned partitions match the canned response,
    // (c) no `CacheHit { source: DispatchedFromOverride }` event fires
    //     (the path is the LLM agent path, not the synthetic
    //     override-shortcircuit),
    // (d) a normal `AgentComplete` event WAS emitted for the dispatch
    //     agent's `agent_id` shape.
    let dir = tempfile::tempdir().unwrap();
    // Intentionally no override files.
    let canned = json!({
        "content": [{
            "type": "text",
            "text": "{\"schema_version\":1,\"subsystems\":[{\"id\":\"agents\",\"members\":[\"foo\"]}]}"
        }]
    });
    let backend = Arc::new(DispatchStagedBackend::new(vec![(
        "dispatch subsystems".to_string(),
        canned,
    )]));
    let runtime = make_runtime(backend.clone() as Arc<dyn LlmBackend>);
    let workspace = AgentsWorkspace::new(dir.path());

    let mut rx = runtime.event_bus.subscribe();

    let partitions = dispatch_subsystems(&runtime, &workspace)
        .await
        .expect("LLM-dispatch path must succeed when backend returns a valid envelope");

    // (b) — returned partitions match canned response.
    assert_eq!(partitions.len(), 1);
    assert_eq!(partitions[0].id, "agents");
    assert_eq!(partitions[0].members, vec!["foo".to_string()]);

    // (a) — backend was actually invoked.
    assert!(
        backend.calls() >= 1,
        "LLM-dispatch path must invoke backend at least once; calls={}",
        backend.calls()
    );

    // (c) + (d) — drain the bus and inspect emitted events.
    let mut saw_dispatched_from_override = false;
    let mut saw_dispatch_agent_complete = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::CacheHit {
                source: CacheHitSource::DispatchedFromOverride,
                ..
            } => {
                saw_dispatched_from_override = true;
            }
            AgentEvent::AgentComplete { agent_id, .. }
                if agent_id.starts_with("dispatch_subsystem::") =>
            {
                saw_dispatch_agent_complete = true;
            }
            _ => {}
        }
    }
    assert!(
        !saw_dispatched_from_override,
        "no `DispatchedFromOverride` cache-hit may fire on the LLM-dispatch path"
    );
    assert!(
        saw_dispatch_agent_complete,
        "expected at least one AgentComplete for the dispatch_subsystem agent"
    );
}

#[tokio::test]
async fn dispatch_with_invalid_override_lane_a_fails() {
    // An override with a duplicate id fails the structural Lane A
    // check (surfaced as `OverrideRequired` in PR-5; recast labels
    // both shapes "lane A").
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(SUBSYSTEMS_OVERRIDE_FILENAME),
        "schema_version: 1\nsubsystems:\n  - id: a\n    members: []\n  - id: a\n    members: []\n",
    )
    .unwrap();
    let backend = Arc::new(CountingBackend::new());
    let runtime = make_runtime(backend.clone() as Arc<dyn LlmBackend>);
    let workspace = AgentsWorkspace::new(dir.path());

    let err = dispatch_subsystems(&runtime, &workspace).await.unwrap_err();
    assert!(
        matches!(err, AgentError::OverrideRequired(ref msg) if msg.contains("duplicate")),
        "expected duplicate-id failure, got {err:?}"
    );
}

#[tokio::test]
async fn dispatch_cache_key_invalidates_when_override_added() {
    // PR-5 cache-invariant rule (recast §6.1): the no-override
    // fingerprint must differ from the with-override fingerprint.
    let backend = Arc::new(CountingBackend::new());
    let runtime = make_runtime(backend.clone() as Arc<dyn LlmBackend>);

    let no_override =
        dispatch_fingerprint(&runtime, AgentStage::DispatchSubsystem, "_workspace", None);
    let with_override = dispatch_fingerprint(
        &runtime,
        AgentStage::DispatchSubsystem,
        "_workspace",
        Some([1u8; 32]),
    );
    assert_ne!(
        no_override.to_cache_key(),
        with_override.to_cache_key(),
        "adding an override (None -> Some) must change the cache key"
    );
}

#[tokio::test]
async fn dispatch_cache_key_invalidates_when_override_removed() {
    // The inverse of the previous test: a synthetic-from-override
    // entry has `Some(sha)`; removing the override yields `None`. The
    // two fingerprints must differ.
    let backend = Arc::new(CountingBackend::new());
    let runtime = make_runtime(backend.clone() as Arc<dyn LlmBackend>);

    let with_override = dispatch_fingerprint(
        &runtime,
        AgentStage::DispatchSubsystem,
        "_workspace",
        Some([2u8; 32]),
    );
    let after_remove =
        dispatch_fingerprint(&runtime, AgentStage::DispatchSubsystem, "_workspace", None);
    assert_ne!(
        with_override.to_cache_key(),
        after_remove.to_cache_key(),
        "removing an override (Some -> None) must change the cache key"
    );
}

#[tokio::test]
async fn dispatch_components_shortcircuit_emits_dispatched_from_override() {
    let dir = tempfile::tempdir().unwrap();
    write_minimal_overrides(dir.path());

    let backend = Arc::new(CountingBackend::new());
    let runtime = make_runtime(backend.clone() as Arc<dyn LlmBackend>);
    let workspace = AgentsWorkspace::new(dir.path());

    let subsystem = SubsystemPartition {
        id: "agents".to_string(),
        members: vec!["foo".to_string()],
    };

    let mut rx = runtime.event_bus.subscribe();

    let components = dispatch_components(&runtime, &workspace, &subsystem)
        .await
        .unwrap();
    assert_eq!(components.len(), 1);
    assert_eq!(components[0].id, "foo");

    let mut hits = 0u32;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::CacheHit { source, .. } = ev {
            if matches!(source, CacheHitSource::DispatchedFromOverride) {
                hits += 1;
            }
        }
    }
    assert_eq!(
        hits, 1,
        "exactly one DispatchedFromOverride CacheHit must fire per dispatch_components call"
    );
    assert_eq!(backend.calls(), 0);
}
