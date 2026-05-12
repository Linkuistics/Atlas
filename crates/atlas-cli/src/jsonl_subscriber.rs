//! JSON-Lines event-stream subscriber.
//!
//! Two destinations:
//!
//! - `JsonlDest::Stdout` — one event per line on stdout, used when
//!   `--no-tui` is set (or stdout is not a terminal).
//! - `JsonlDest::File(path)` — one event per line in a log file, used
//!   when `--log-events <PATH>` is set. Active *in parallel* with the
//!   TUI subscriber (PR-6) for post-hoc analysis.
//!
//! Holds the drain-handshake invariant: `RuntimeComplete` is the
//! sentinel; the subscriber flushes its sink, signals `done_tx`, and
//! returns. The runtime (PR-4) `try_join!`s all subscribers' `done_rx`
//! futures before returning from `run()`.
//!
//! Lagged-receiver handling: emits a sentinel `{"event":"LaggedReceiver","dropped":N}`
//! line so the lag is visible in the log stream. Silent drop is
//! forbidden (subscriber discipline; see `agent_cache_writer.rs` for
//! the same rule).

use std::io::Write;
use std::path::PathBuf;

use atlas_agents::{AgentEvent, EventBus};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;

/// Where the JSON-Lines subscriber writes events.
pub enum JsonlDest {
    Stdout,
    File(PathBuf),
}

/// Run the JSON-Lines subscriber to completion. Blocks on the bus
/// until [`AgentEvent::RuntimeComplete`] is observed, then flushes the
/// sink and signals `done_tx`.
pub async fn run(bus: &EventBus, dest: JsonlDest, done_tx: oneshot::Sender<()>) {
    let mut rx = bus.subscribe();
    let mut sink: Box<dyn Write + Send> = match dest {
        JsonlDest::Stdout => Box::new(std::io::stdout()),
        JsonlDest::File(p) => match std::fs::File::create(&p) {
            Ok(f) => Box::new(f),
            Err(err) => {
                // The file could not be opened. Surface the failure
                // and signal completion immediately so the runtime
                // doesn't deadlock waiting for us. Subsequent events
                // are not captured.
                eprintln!("atlas: failed to open events log {}: {err}", p.display());
                let _ = done_tx.send(());
                return;
            }
        },
    };
    loop {
        match rx.recv().await {
            Ok(AgentEvent::RuntimeComplete) => {
                let _ = sink.flush();
                let _ = done_tx.send(());
                return;
            }
            Ok(event) => {
                // serde_json::to_string returns Err only for a Value
                // graph that contains a non-string map key; AgentEvent
                // has none. Tolerate failure rather than panicking.
                match serde_json::to_string(&event) {
                    Ok(line) => {
                        let _ = writeln!(sink, "{line}");
                    }
                    Err(err) => {
                        eprintln!("atlas: failed to serialise event: {err}");
                    }
                }
            }
            Err(RecvError::Lagged(n)) => {
                let _ = writeln!(sink, r#"{{"event":"LaggedReceiver","dropped":{n}}}"#);
            }
            Err(RecvError::Closed) => return,
        }
    }
}
