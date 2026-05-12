//! PR-5 dispatch-shortcircuit acceptance tests (plan §4 Task 5.5).
//!
//! Verifies the override-file shortcircuit in `dispatch_subsystems` /
//! `dispatch_components`:
//!
//! - With an override present: `CacheHit { source:
//!   DispatchedFromOverride }` fires; the parsed partitions match the
//!   override file; the LLM dispatch agent does NOT fire.
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

/// Test stub backend. The dispatch shortcircuit path never invokes
/// the backend — every test asserts the backend's call counter stays
/// at zero. Constructing a real `StagedBackend` would be misleading
/// since the PR-5 dispatch never reaches it under override-present.
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
async fn dispatch_without_override_file_surfaces_override_required_in_pr5() {
    // PR-5 ships the override-shortcircuit + the cache-invariant
    // fingerprint contributor; PR-7 wires the production
    // LLM-dispatch agent. In PR-5 the no-override path surfaces
    // `OverrideRequired` rather than firing an LLM call (per the
    // module doc rationale).
    let dir = tempfile::tempdir().unwrap();
    // Intentionally no override files.
    let backend = Arc::new(CountingBackend::new());
    let runtime = make_runtime(backend.clone() as Arc<dyn LlmBackend>);
    let workspace = AgentsWorkspace::new(dir.path());

    let err = dispatch_subsystems(&runtime, &workspace).await.unwrap_err();
    assert!(
        matches!(err, AgentError::OverrideRequired(_)),
        "expected OverrideRequired, got {err:?}"
    );
    assert_eq!(
        backend.calls(),
        0,
        "no backend call should fire on the OverrideRequired path"
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
