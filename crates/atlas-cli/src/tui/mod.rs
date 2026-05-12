//! `ratatui` TUI subscriber.
//!
//! Subscribes to the `AgentEvent` bus and renders a four-widget frame
//! at ~20 Hz (50 ms tick). Composes:
//!
//! - [`tree_view`] (top, flexible) — workspace → agent tree
//! - [`token_panel`] (middle row, left half) — running token totals
//! - [`iteration_bar`] (middle row, right half) — iteration counter
//!   + convergence indicator
//! - [`stuck_detect`] (bottom 2 lines) — 90s stuck-agent heuristic
//!
//! The subscriber holds the drain-handshake invariant: when
//! `AgentEvent::RuntimeComplete` arrives it cleans up the terminal,
//! signals `done_tx`, and returns.
//!
//! ## Terminal cleanup on panic
//!
//! Raw mode and the alternate screen are entered before any render
//! call and must be released even if the renderer panics. The cleanup
//! is wrapped in a [`TerminalGuard`] newtype whose `Drop` runs the
//! cleanup unconditionally; combined with the runtime's
//! `catch_unwind`-or-`AbortHandle` discipline (PR-7 wires the
//! supervising boundary), this ensures the user's terminal is never
//! left raw after a TUI panic.

use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;
use ratatui::Terminal;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

use atlas_agents::{AgentEvent, EventBus, Subscriber};

pub mod iteration_bar;
pub mod state;
pub mod stuck_detect;
pub mod token_panel;
pub mod tree_view;

pub use state::{TuiSnapshot, TuiState};

/// Configuration knobs for the TUI subscriber.
#[derive(Debug, Clone, Copy, Default)]
pub struct TuiConfig {
    /// When `true`, the token panel shows a per-provider breakdown.
    /// Wired from the `--tui-show-providers` CLI flag.
    pub show_providers: bool,
}

/// RAII guard that owns the terminal's raw-mode + alternate-screen
/// state. Constructing it enters raw mode + the alternate screen; its
/// `Drop` impl restores both. Wrap the `Terminal` lifetime in this so
/// even a render-time panic walks back through `Drop` and the user's
/// terminal returns to a sane state.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort: a failure here means something else has already
        // mangled the terminal; we can't recover it.
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    }
}

/// Run the TUI subscriber.
///
/// Subscribes to `bus`, mutates `state` from incoming events at
/// ~20 Hz, and renders a frame on every tick. Returns after observing
/// `RuntimeComplete` (or after the bus closes), signalling `done_tx`
/// once the terminal is restored.
///
/// `config.show_providers` toggles the per-provider token breakdown
/// in the token panel.
pub async fn run(
    bus: &EventBus,
    state: Arc<Mutex<TuiState>>,
    config: TuiConfig,
    done_tx: oneshot::Sender<()>,
) -> std::io::Result<()> {
    let rx = bus.subscribe();
    run_with_subscriber(rx, state, config, done_tx).await
}

/// Variant of [`run`] that takes a pre-existing subscriber. Used by
/// `replay::run_with_tui` so the subscriber is registered before any
/// emit happens (tokio's broadcast channel only delivers events
/// emitted after `subscribe()` returns; without the pre-subscribe
/// the spawned TUI task can race the first `bus.emit()` and lose
/// events).
pub async fn run_with_subscriber(
    mut rx: Subscriber,
    state: Arc<Mutex<TuiState>>,
    config: TuiConfig,
    done_tx: oneshot::Sender<()>,
) -> std::io::Result<()> {
    // Wrap the (raw-mode-entry + terminal-build) prelude so a
    // headless environment (no controlling TTY, e.g. `cargo test`)
    // still signals `done_tx` before propagating the error. Without
    // this the bus emitter blocks forever on `done_rx.await` in
    // `replay::run_with_tui`.
    let guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(err) => {
            // Drain the bus to the end so the emitter never blocks
            // on a back-pressure boundary, then signal done_tx.
            drain_to_complete(&mut rx, &state).await;
            let _ = done_tx.send(());
            return Err(err);
        }
    };
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(err) => {
            // `guard` drops here, restoring the terminal.
            drop(guard);
            drain_to_complete(&mut rx, &state).await;
            let _ = done_tx.send(());
            return Err(err);
        }
    };

    'outer: loop {
        tokio::select! {
            event = rx.recv() => {
                // PR-7 (PR-6 closeout MEDIUM-3): drain every
                // immediately-ready event before yielding back to
                // the sleep arm. On a large replay burst (200+
                // agents), the prior one-event-per-50ms-tick shape
                // serialised to ~10s of busy spinning while the
                // TUI lagged behind reality. With the drain, the
                // sleep arm fires once events drain; the redraw
                // sees the up-to-date state.
                let drained = match event {
                    Ok(AgentEvent::RuntimeComplete) => {
                        state.lock().await.apply(AgentEvent::RuntimeComplete);
                        break 'outer;
                    }
                    Ok(e) => {
                        let mut guard = state.lock().await;
                        guard.apply(e);
                        // Drain non-blocking-ready events; loop
                        // breaks on `RuntimeComplete` so the
                        // redraw arm cannot starve.
                        let mut should_break = false;
                        while let Ok(extra) = rx.try_recv() {
                            if matches!(extra, AgentEvent::RuntimeComplete) {
                                guard.apply(AgentEvent::RuntimeComplete);
                                should_break = true;
                                break;
                            }
                            guard.apply(extra);
                        }
                        should_break
                    }
                    Err(RecvError::Lagged(n)) => {
                        state.lock().await.note_lag(n);
                        false
                    }
                    Err(RecvError::Closed) => break 'outer,
                };
                if drained {
                    break 'outer;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                let snapshot = {
                    let s = state.lock().await;
                    s.clone()
                };
                // `terminal.draw` returns an io::Result; on render
                // failure log and continue rather than tearing the
                // run down.
                let draw_result = terminal.draw(|f| render_frame(f, &snapshot, config));
                if let Err(err) = draw_result {
                    eprintln!("atlas: tui draw failed: {err}");
                }
            }
        }
    }

    drop(terminal);
    drop(guard);
    let _ = done_tx.send(());
    Ok(())
}

/// Drain the bus to `RuntimeComplete` (or `Closed`), applying each
/// event to state. Used when the TUI subscriber fails to enter raw
/// mode (e.g. headless test environment) — we still must consume
/// events so the emitter doesn't see backpressure and so state ends
/// in a sane shape (`runtime_complete = true`).
async fn drain_to_complete(rx: &mut Subscriber, state: &Arc<Mutex<TuiState>>) {
    loop {
        match rx.recv().await {
            Ok(AgentEvent::RuntimeComplete) => {
                state.lock().await.apply(AgentEvent::RuntimeComplete);
                return;
            }
            Ok(e) => state.lock().await.apply(e),
            Err(RecvError::Lagged(n)) => state.lock().await.note_lag(n),
            Err(RecvError::Closed) => return,
        }
    }
}

/// Compose the four widgets into a single frame.
///
/// Layout: `[ tree | tokens-iteration | health ]`
/// stacked vertically with `[ Min(0), Length(3), Length(2) ]` (plan §4
/// Task 6 Step 6.2). The token + iteration panels share the middle
/// row 50/50.
pub fn render_frame(frame: &mut Frame, state: &TuiState, config: TuiConfig) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(frame.area());

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[1]);

    tree_view::render(frame, outer[0], state);
    token_panel::render(frame, middle[0], state, config.show_providers);
    iteration_bar::render(frame, middle[1], state);
    stuck_detect::render(frame, outer[2], state);
}

#[cfg(test)]
mod tests {
    /// Verify the TerminalGuard's `Drop` impl runs even on panic.
    /// We can't actually enter raw mode in `cargo test` (no real TTY)
    /// so the test exercises the cleanup path symbolically: we
    /// construct a struct holding a `Drop` we want to fire, panic
    /// inside `catch_unwind`, and confirm the drop ran.
    ///
    /// This proves the *structural* pattern; the real raw-mode
    /// cleanup falls out of [`TerminalGuard::drop`] for free.
    #[test]
    fn drop_guard_cleanup_runs_on_panic() {
        use std::cell::Cell;
        use std::panic::AssertUnwindSafe;

        struct TestGuard<'a>(&'a Cell<bool>);
        impl Drop for TestGuard<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let cleaned = Cell::new(false);
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _g = TestGuard(&cleaned);
            panic!("simulated tui panic");
        }));
        assert!(result.is_err(), "panic must propagate to catch_unwind");
        assert!(
            cleaned.get(),
            "TerminalGuard-shaped Drop must fire on panic so the \
             user's terminal returns to a sane state"
        );
    }
}
