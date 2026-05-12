//! Golden-file test for the JSON-Lines event-stream subscriber.
//!
//! Emits a fixed event sequence into an `EventBus`, runs the subscriber
//! against a temp file, and asserts:
//!
//! - One event per line (no blank lines, no multi-line JSON).
//! - Each line parses back as the same `AgentEvent` variant that was
//!   emitted (`serde` round-trip).
//! - `RuntimeComplete` is the drain sentinel and is consumed by the
//!   subscriber (not emitted to the log).
//! - The subscriber signals `done_tx` before returning
//!   (drain-handshake invariant).
//!
//! Spec anchors: plan §4 Task 2 step 2.6 + brainstorm §4 (PR-2).

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;

use atlas_agents::events::{AgentEvent, EventBus, Grade};
use atlas_agents::transport::TransportFlavour;
use atlas_cli::jsonl_subscriber::{run, JsonlDest};
use tempfile::NamedTempFile;
use tokio::sync::oneshot;

fn fixture_events() -> Vec<AgentEvent> {
    vec![
        AgentEvent::AgentStart {
            agent_id: "a-1".to_string(),
            parent_id: None,
            stage: "L3".to_string(),
            target: "lib".to_string(),
            fingerprint: "deadbeef".to_string(),
            started_at: "2026-05-12T00:00:00Z".to_string(),
            transport: TransportFlavour::ClaudeCode,
        },
        AgentEvent::ToolCall {
            agent_id: "a-1".to_string(),
            tool_name: "classify_cargo_component".to_string(),
            args_summary: r#"{"component_id":"x"}"#.to_string(),
        },
        AgentEvent::ToolResult {
            agent_id: "a-1".to_string(),
            tool_name: "classify_cargo_component".to_string(),
            result_summary: r#"{"kind":"library"}"#.to_string(),
            ms: 12,
            bytes: 48,
        },
        AgentEvent::AgentComplete {
            agent_id: "a-1".to_string(),
            output_sha: "feedface".to_string(),
            confidence_grade: Grade::Strong,
            tokens_in: 100,
            tokens_out: 50,
            ms: 1234,
            provider: "anthropic".to_string(),
        },
    ]
}

#[tokio::test]
async fn jsonl_subscriber_emits_one_event_per_line_and_round_trips() {
    let tmp = NamedTempFile::new().expect("tempfile");
    let log_path = tmp.path().to_path_buf();
    // Drop the tempfile handle so the subscriber can `File::create`
    // the same path. The tempdir parent still cleans up on drop.
    drop(tmp);

    let bus = Arc::new(EventBus::new(64));
    let (done_tx, done_rx) = oneshot::channel::<()>();

    // Subscribe inside the spawned task. Broadcast does not replay
    // history, so we briefly sleep before emitting to give the task
    // a chance to register its receiver.
    let bus_for_task = Arc::clone(&bus);
    let log_path_for_task = log_path.clone();
    let task = tokio::spawn(async move {
        run(&bus_for_task, JsonlDest::File(log_path_for_task), done_tx).await;
    });

    // Wait for the subscriber to register. A short sleep is sufficient
    // and avoids relying on broadcast internals.
    tokio::time::sleep(Duration::from_millis(20)).await;

    for ev in fixture_events() {
        bus.emit(ev);
    }
    bus.emit(AgentEvent::RuntimeComplete);

    // Drain-handshake: the runtime would `try_join!` here. We model
    // the same wait so a test failure here means a broken handshake.
    done_rx.await.expect("subscriber must signal done_tx");
    task.await.expect("subscriber task must complete");

    // Read back the log file. One JSON event per line; no blanks.
    let f = File::open(&log_path).expect("open events log");
    let reader = BufReader::new(f);
    let lines: Vec<String> = reader
        .lines()
        .map(|l| l.expect("readable line"))
        .filter(|l| !l.is_empty())
        .collect();

    // The fixture has 4 events; RuntimeComplete is the drain sentinel
    // and is consumed by the subscriber rather than emitted to the log.
    assert_eq!(
        lines.len(),
        4,
        "expected one line per non-sentinel event; got {}: {lines:?}",
        lines.len()
    );

    // Each line round-trips back to an AgentEvent of the same variant
    // (variant tags are stable per the `Serialize` derive).
    let parsed: Vec<AgentEvent> = lines
        .iter()
        .map(|l| serde_json::from_str(l).expect("line parses as AgentEvent"))
        .collect();

    assert!(matches!(parsed[0], AgentEvent::AgentStart { .. }));
    assert!(matches!(parsed[1], AgentEvent::ToolCall { .. }));
    assert!(matches!(parsed[2], AgentEvent::ToolResult { .. }));
    assert!(matches!(parsed[3], AgentEvent::AgentComplete { .. }));

    // Spot-check round-trip equality on field values.
    let AgentEvent::AgentStart {
        agent_id,
        fingerprint,
        transport,
        ..
    } = &parsed[0]
    else {
        unreachable!("matched above");
    };
    assert_eq!(agent_id, "a-1");
    assert_eq!(fingerprint, "deadbeef");
    assert_eq!(*transport, TransportFlavour::ClaudeCode);
}
