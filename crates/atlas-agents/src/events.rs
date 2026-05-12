//! Runtime event bus + `AgentEvent` enum.
//!
//! The event bus is the spine for cross-cutting observability of the
//! LLM-spine runtime: every agent invocation emits a structured stream
//! of events that subscribers — the persistent transcript-cache writer,
//! the JSON-Lines log subscriber, and (PR-6) the TUI — consume in
//! parallel. The transport is `tokio::sync::broadcast`; the capacity
//! ships at 1024 per brainstorm §2 row 10.
//!
//! See the LLM-spine recast spec §9.1 for the event-type catalogue and
//! the drain-handshake protocol.
//!
//! # Drain handshake
//!
//! Subscribers must process [`AgentEvent::RuntimeComplete`] and signal
//! their `done_tx: oneshot::Sender<()>` before returning. The runtime
//! (PR-4) `try_join!`s all the `done_rx` futures before exiting `run()`,
//! so that `AgentComplete` events queued behind a slow subscriber are
//! flushed before the runtime tears down. Silent drop of an event would
//! corrupt the transcript cache (the writer is downstream of
//! `AgentComplete`), so lagged-receiver handling is *error-and-log*, not
//! silent-drop — see each subscriber implementation.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::transport::TransportFlavour;

/// Confidence grade attached to an `AgentComplete` event. The spine
/// uses these to decide whether to fire an audit (recast §9.1, §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grade {
    Strong,
    Moderate,
    Weak,
    Declines,
}

/// Provenance label for a `CacheHit` event. Distinguishes a true
/// transcript-cache replay from a dispatch that short-circuited via an
/// override pin (recast §6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheHitSource {
    AgentCache,
    DispatchedFromOverride,
}

/// Structured event emitted by the agent runtime. One variant per
/// observable transition; subscribers pattern-match on the variants
/// they care about.
///
/// `RuntimeComplete` is the drain-handshake sentinel: it is the last
/// event the runtime emits, and every subscriber that participates in
/// the handshake must process it before returning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    IterationBoundary {
        iter: u32,
        prior_model_sha: Option<String>,
    },
    AgentStart {
        agent_id: String,
        parent_id: Option<String>,
        stage: String,
        target: String,
        fingerprint: String,
        started_at: String,
        transport: TransportFlavour,
    },
    ToolCall {
        agent_id: String,
        tool_name: String,
        args_summary: String,
    },
    ToolResult {
        agent_id: String,
        tool_name: String,
        result_summary: String,
        ms: u64,
        bytes: u64,
    },
    AgentComplete {
        agent_id: String,
        output_sha: String,
        confidence_grade: Grade,
        tokens_in: u64,
        tokens_out: u64,
        ms: u64,
        provider: String,
    },
    AuditFire {
        agent_id: String,
        audit_reason: String,
        auditor_provider: String,
    },
    AuditVerdict {
        agent_id: String,
        verdict: String,
    },
    AuditDegraded {
        reason: String,
    },
    HardFail {
        agent_id: String,
        error_kind: String,
        error_summary: String,
        retry_count: u32,
    },
    CacheHit {
        agent_id: String,
        fingerprint: String,
        replayed_at: String,
        source: CacheHitSource,
    },
    /// Drain-handshake sentinel. The runtime emits this last; each
    /// subscriber processes it and signals `done_tx` before returning.
    RuntimeComplete,
}

/// Broadcast-channel event bus. `Clone`-able fan-out to many subscribers;
/// each subscriber has its own backpressure buffer of `capacity` events.
///
/// Capacity is set at construction (1024 in production per brainstorm
/// §2 row 10). When a subscriber falls behind by more than `capacity`
/// events the broadcast receiver returns
/// [`tokio::sync::broadcast::error::RecvError::Lagged`] on the next
/// `recv().await`; subscribers must log this (so the lag is visible)
/// and continue — they MUST NOT silently drop, because `AgentComplete`
/// events drive transcript-cache writes downstream.
pub struct EventBus {
    tx: broadcast::Sender<AgentEvent>,
}

/// Re-export of [`broadcast::Receiver`] under a domain-named alias.
/// Subscribers use this name in signatures; the underlying type is the
/// stock tokio receiver and exposes its full API surface.
pub type Subscriber = broadcast::Receiver<AgentEvent>;

impl EventBus {
    /// Construct a bus with `capacity` events of buffering per
    /// subscriber. Use [`EventBus::with_default_capacity`] in
    /// production; pick a small `capacity` (e.g. 64) in tests where
    /// lag/backpressure is irrelevant.
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Production-defaulted capacity (1024).
    pub fn with_default_capacity() -> Self {
        Self::new(1024)
    }

    /// Open a new subscriber. The receiver only sees events emitted
    /// after this call returns — late subscribers do not replay history.
    pub fn subscribe(&self) -> Subscriber {
        self.tx.subscribe()
    }

    /// Best-effort emit. A failed send (no live receivers) is silently
    /// ignored — the runtime emits events unconditionally, and a missing
    /// subscriber is the caller's choice (e.g. `--no-tui` without
    /// `--log-events`).
    pub fn emit(&self, event: AgentEvent) {
        let _ = self.tx.send(event);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::with_default_capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscriber_receives_emitted_events_in_order() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();

        bus.emit(AgentEvent::IterationBoundary {
            iter: 0,
            prior_model_sha: None,
        });
        bus.emit(AgentEvent::RuntimeComplete);

        let first = rx.recv().await.expect("first event");
        assert!(matches!(
            first,
            AgentEvent::IterationBoundary { iter: 0, .. }
        ));
        let second = rx.recv().await.expect("second event");
        assert!(matches!(second, AgentEvent::RuntimeComplete));
    }

    #[tokio::test]
    async fn emit_without_subscribers_does_not_error() {
        // Production code emits unconditionally; if no subscriber is
        // attached we still want a clean run.
        let bus = EventBus::new(4);
        bus.emit(AgentEvent::RuntimeComplete);
    }

    #[tokio::test]
    async fn lagged_subscriber_observes_recv_error_lagged() {
        // Capacity 2 — emit 4 events while the receiver waits, then
        // recv() should report a Lagged(n) error. Validates that the
        // bus surfaces backpressure rather than silently dropping.
        let bus = EventBus::new(2);
        let mut rx = bus.subscribe();
        for _ in 0..4 {
            bus.emit(AgentEvent::RuntimeComplete);
        }
        let err = rx.recv().await.expect_err("expected Lagged error");
        match err {
            broadcast::error::RecvError::Lagged(n) => assert!(n >= 1, "lag count = {n}"),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}
