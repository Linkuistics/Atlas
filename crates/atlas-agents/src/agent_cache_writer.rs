//! Subscriber that materialises transcript-cache entries from
//! `AgentComplete` events.
//!
//! This file is the runtime-side of the Phase 7 PR-2 transcript-cache
//! plumbing. The cache itself (`atlas_engine::LlmResponseCache`,
//! `call_agent_cached`) lives in `atlas-engine`; the subscriber lives
//! here because it consumes `AgentEvent`s and `atlas-engine` cannot
//! depend on `atlas-agents` (cycle — `atlas-agents` already depends
//! on `atlas-engine`).
//!
//! Spawned by the runtime alongside the runtime-execution task. Holds
//! the drain-handshake invariant: `RuntimeComplete` is the sentinel;
//! the subscriber processes it and signals `done_tx: oneshot::Sender<()>`
//! before returning. The runtime (PR-4) `try_join!`s all subscriber
//! `done_rx` futures before returning from `run()`.
//!
//! ## PR-2 scope (stub)
//!
//! For PR-2 the body of the `AgentComplete` match arm is structurally
//! enforced (the file compiles and the signature is right) but the
//! actual cache-write call is a no-op — PR-2's transcript-cache layer
//! is not yet wired to the runtime, PR-4 wires it. The TODO marker
//! below is the wire-up point.

use std::sync::Arc;

use atlas_engine::LlmResponseCache;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;

use crate::events::{AgentEvent, EventBus};

/// Run the transcript-cache writer subscriber to completion.
///
/// Blocks (`.await`) on the bus until [`AgentEvent::RuntimeComplete`]
/// is observed; then signals `done_tx` and returns. Lagged-receiver
/// events are logged (NOT silently dropped) because `AgentComplete`
/// drives cache writes — a silent drop would corrupt the cache.
///
/// `cache` is held as `Arc` so the runtime can share it with other
/// subscribers and with the synchronous engine code that reads from
/// the same cache instance.
pub async fn run(bus: &EventBus, cache: Arc<LlmResponseCache>, done_tx: oneshot::Sender<()>) {
    let mut rx = bus.subscribe();
    let _cache = cache; // silence dead-code until PR-4 wires the writer.
    loop {
        match rx.recv().await {
            Ok(AgentEvent::AgentComplete {
                agent_id,
                output_sha,
                ..
            }) => {
                // TODO(PR-4): materialise the transcript-cache entry
                // for this agent invocation. PR-2 ships the subscriber
                // shape only; the runtime that emits `AgentComplete`
                // with a correlated transcript-accumulator handle
                // lands in PR-4. At that point the writer correlates
                // by `agent_id`, retrieves the accumulated
                // `(transcript_bytes, output_bytes)`, builds the
                // `AgentInputFingerprint` from the prior `AgentStart`,
                // and calls `LlmResponseCache::call_agent_cached` with
                // a `compute` closure that returns the already-known
                // result (so the cache layer writes the pair on the
                // success arm).
                let _ = (agent_id, output_sha);
            }
            Ok(AgentEvent::RuntimeComplete) => {
                // Drain-handshake: signal completion before returning
                // so the runtime can `try_join!` us and know we've
                // flushed every queued AgentComplete.
                let _ = done_tx.send(());
                return;
            }
            Ok(_) => continue,
            Err(RecvError::Lagged(n)) => {
                // Lagged receivers are an error, not a silent drop.
                // A dropped `AgentComplete` would leave the
                // transcript cache short an entry and force a needless
                // re-run on the next pass — strictly worse than the
                // log line, so we surface it loudly.
                tracing::error!(
                    lagged = n,
                    "agent_cache_writer lagged; dropping events would corrupt cache"
                );
                continue;
            }
            Err(RecvError::Closed) => return,
        }
    }
}
