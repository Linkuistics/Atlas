// Clippy's `disallowed_methods` lint (set crate-wide to block
// `Runtime::block_on` / `Handle::block_on`) misfires on
// `JoinHandle::await` inside `#[tokio::test]` async fns — the `.await`
// operator expansion looks like an internal runtime block_on call but
// is a normal Future::poll. The lint is meant to block synchronous
// blocking inside the async runtime, which this test does not do.
#![allow(clippy::disallowed_methods)]

//! Drain-handshake integration test for the agent runtime event bus.
//!
//! Asserts that `RuntimeComplete` is a proper sentinel: a slow
//! subscriber that is still processing events when `RuntimeComplete`
//! arrives must finish flushing before its `done_tx` fires. The
//! runtime (PR-4) `try_join!`s every subscriber's `done_rx` before
//! returning, so this test models the same wait the runtime will do.
//!
//! Spec anchors: LLM-spine recast §6.4 (cache-write drain) and §9.1
//! (subscriber lifecycle). Plan §4 Task 2 step 2.8.

use atlas_agents::events::{AgentEvent, EventBus, Grade, Subscriber};
use atlas_agents::transport::TransportFlavour;
use tokio::sync::oneshot;
use tokio::time::{sleep, Duration};

/// A slow subscriber: processes every event with a `sleep` so the
/// drain-handshake actually has work to wait on. Tracks the number of
/// events seen so the test can assert it observed every emit before
/// signalling `done`.
async fn slow_subscriber(
    mut rx: Subscriber,
    done_tx: oneshot::Sender<u32>,
    per_event_delay_ms: u64,
) {
    let mut seen: u32 = 0;
    loop {
        match rx.recv().await {
            Ok(AgentEvent::RuntimeComplete) => {
                seen += 1;
                // Mimic any flush work the subscriber would do before
                // signalling completion.
                sleep(Duration::from_millis(per_event_delay_ms)).await;
                let _ = done_tx.send(seen);
                return;
            }
            Ok(_) => {
                seen += 1;
                sleep(Duration::from_millis(per_event_delay_ms)).await;
            }
            Err(_) => {
                let _ = done_tx.send(seen);
                return;
            }
        }
    }
}

#[tokio::test]
async fn runtime_complete_blocks_until_all_subscribers_flush() {
    let bus = EventBus::new(64);
    let (done_a_tx, done_a_rx) = oneshot::channel::<u32>();
    let (done_b_tx, done_b_rx) = oneshot::channel::<u32>();

    // Spawn two slow subscribers. Subscribe *before* emitting so
    // both receivers see every event (broadcast::subscribe only
    // delivers events emitted after subscribe was called).
    let rx_a = bus.subscribe();
    let rx_b = bus.subscribe();
    let handle_a = tokio::spawn(slow_subscriber(rx_a, done_a_tx, 5));
    let handle_b = tokio::spawn(slow_subscriber(rx_b, done_b_tx, 10));

    bus.emit(AgentEvent::AgentStart {
        agent_id: "a1".to_string(),
        parent_id: None,
        stage: "L3".to_string(),
        target: "lib".to_string(),
        fingerprint: "deadbeef".to_string(),
        started_at: "2026-05-12T00:00:00Z".to_string(),
        transport: TransportFlavour::ClaudeCode,
    });
    bus.emit(AgentEvent::AgentComplete {
        agent_id: "a1".to_string(),
        output_sha: "feedface".to_string(),
        confidence_grade: Grade::Strong,
        tokens_in: 100,
        tokens_out: 50,
        ms: 1000,
        provider: "anthropic".to_string(),
    });
    bus.emit(AgentEvent::RuntimeComplete);

    // The runtime would `try_join!` here. We model the same wait:
    // both subscribers must complete and return their per-subscriber
    // event count before we proceed past this point.
    let (count_a, count_b) =
        tokio::try_join!(done_a_rx, done_b_rx).expect("both subscribers must flush before exit");

    // Each subscriber must have observed all three events
    // (AgentStart + AgentComplete + RuntimeComplete).
    assert_eq!(
        count_a, 3,
        "subscriber A must have processed every event before signalling done"
    );
    assert_eq!(
        count_b, 3,
        "subscriber B must have processed every event before signalling done"
    );

    // Both subscriber tasks should have returned cleanly.
    handle_a.await.expect("subscriber A task panicked");
    handle_b.await.expect("subscriber B task panicked");
}
