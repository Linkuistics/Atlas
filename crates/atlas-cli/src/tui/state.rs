//! TUI state model.
//!
//! `TuiState` is the live, in-memory projection of the `AgentEvent`
//! stream into a shape the four widget modules render. Holds the
//! workspace → subsystem → component → agent tree, the running token
//! totals (per-provider), the current iteration counter, the stuck
//! detector, and a `lag` counter that records cumulative
//! `RecvError::Lagged(n)` observations.
//!
//! [`TuiSnapshot`] is a serde-serialisable view of state — the PR-6
//! replay test asserts that the snapshot taken from a live run and the
//! snapshot taken from `--replay-from-cache` are byte-equal for the
//! same event sequence (recast §9.2 acceptance gate).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use atlas_agents::events::CacheHitSource;
use atlas_agents::{AgentEvent, Grade, Provider, TransportFlavour};
use serde::{Deserialize, Serialize};

/// Status of a single agent within the workspace tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    /// `AgentStart` observed; no terminal event yet.
    Running,
    /// `AgentComplete` observed.
    Complete { grade: Grade },
    /// `HardFail` observed.
    HardFailed { error_kind: String },
    /// `CacheHit` observed.
    CacheHit { source: CacheHitSource },
}

/// Per-agent record carried inside [`Tree`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentNode {
    pub agent_id: String,
    pub stage: String,
    pub target: String,
    pub transport: TransportFlavour,
    pub status: AgentStatus,
}

/// Hierarchical projection of the live workspace.
///
/// `tree` is a stage → list-of-agents map. The recast spec's
/// workspace → subsystem → component → agent hierarchy is encoded
/// flat here under the `stage` axis: PR-6 ships the rendering surface;
/// PR-7's runtime-wiring step refines the projection (e.g. group by
/// subsystem within each stage).
///
/// `BTreeMap` for the stage axis so iteration order is deterministic
/// (load-bearing for snapshot byte-equality in the replay test).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub by_stage: BTreeMap<String, Vec<AgentNode>>,
}

impl Tree {
    fn upsert(&mut self, node: AgentNode) {
        let stage = node.stage.clone();
        let bucket = self.by_stage.entry(stage).or_default();
        if let Some(existing) = bucket.iter_mut().find(|n| n.agent_id == node.agent_id) {
            *existing = node;
        } else {
            bucket.push(node);
        }
    }

    fn set_status(&mut self, agent_id: &str, status: AgentStatus) {
        for bucket in self.by_stage.values_mut() {
            if let Some(existing) = bucket.iter_mut().find(|n| n.agent_id == agent_id) {
                existing.status = status;
                return;
            }
        }
    }
}

/// Running token totals, broken out by [`Provider`] so the
/// `--tui-show-providers` flag can render a per-provider line in the
/// token panel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTotals {
    /// `BTreeMap` for snapshot-determinism (same reason as
    /// [`Tree::by_stage`]).
    pub by_provider: BTreeMap<Provider, ProviderTotals>,
}

/// Per-provider token totals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTotals {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

impl TokenTotals {
    fn record(&mut self, provider_str: &str, tokens_in: u64, tokens_out: u64) {
        let provider = match provider_str {
            "Anthropic" | "anthropic" => Provider::Anthropic,
            "OpenAi" | "openai" | "OpenAI" => Provider::OpenAi,
            // Unknown provider strings cluster under Anthropic by
            // default — this is forensic-only data; the snapshot view
            // never round-trips to wire and a bad bucket assignment
            // surfaces in the panel as visible noise.
            _ => Provider::Anthropic,
        };
        let bucket = self.by_provider.entry(provider).or_default();
        bucket.tokens_in += tokens_in;
        bucket.tokens_out += tokens_out;
    }

    /// Sum of every provider's `tokens_in`.
    pub fn total_in(&self) -> u64 {
        self.by_provider.values().map(|t| t.tokens_in).sum()
    }

    /// Sum of every provider's `tokens_out`.
    pub fn total_out(&self) -> u64 {
        self.by_provider.values().map(|t| t.tokens_out).sum()
    }
}

/// Stuck-agent heuristic. Holds the threshold + a `last_activity`
/// timestamp. The widget renders a warning when
/// `Instant::now() - last_activity > threshold`.
///
/// `Instant` is not serialisable: snapshots elide the field via
/// [`TuiSnapshot::stuck_threshold_secs`] (only the threshold survives
/// the round-trip; the elapsed-time view is rendering-only state and
/// would diverge between live and replay anyway).
#[derive(Debug, Clone)]
pub struct StuckDetector {
    pub threshold: Duration,
    pub last_activity: Instant,
}

impl StuckDetector {
    /// 90s threshold per plan §4 Task 6 Step 6.2.
    pub fn new() -> Self {
        Self {
            threshold: Duration::from_secs(90),
            last_activity: Instant::now(),
        }
    }

    /// Returns `Some(elapsed)` if the elapsed-since-activity exceeds
    /// the threshold; otherwise `None`. The widget uses this to gate
    /// rendering of the warning line.
    pub fn check(&self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.last_activity);
        if elapsed > self.threshold {
            Some(elapsed)
        } else {
            None
        }
    }

    fn note_activity(&mut self) {
        self.last_activity = Instant::now();
    }
}

impl Default for StuckDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Top-level TUI state. Mutated from the event stream; rendered by the
/// four widget modules.
#[derive(Debug, Clone, Default)]
pub struct TuiState {
    pub workspace_tree: Tree,
    pub token_totals: TokenTotals,
    /// Current iteration (most recent `IterationBoundary.iter` seen).
    pub iteration: u32,
    /// Prior iteration's `prior_model_sha`, if any — drives the
    /// iteration-bar convergence indicator. `(latest_iter,
    /// latest_prior_sha, prev_iter, prev_prior_sha)` are the inputs
    /// the iteration-bar renderer needs; PR-5 wires the second
    /// iteration boundary on, PR-6 ships dormant single-iteration
    /// rendering.
    pub last_prior_model_sha: Option<String>,
    pub prev_prior_model_sha: Option<String>,
    pub stuck: StuckDetector,
    pub lag: u64,
    /// `true` once `RuntimeComplete` has been observed.
    pub runtime_complete: bool,
}

impl TuiState {
    /// Apply one event to state. Idempotent in the sense that re-applying
    /// the same sequence to a fresh `TuiState` produces an equal
    /// [`TuiSnapshot`] — load-bearing for the replay-test acceptance gate.
    pub fn apply(&mut self, event: AgentEvent) {
        self.stuck.note_activity();
        match event {
            AgentEvent::IterationBoundary {
                iter,
                prior_model_sha,
            } => {
                self.prev_prior_model_sha = self.last_prior_model_sha.take();
                self.last_prior_model_sha = prior_model_sha;
                self.iteration = iter;
            }
            AgentEvent::AgentStart {
                agent_id,
                stage,
                target,
                transport,
                ..
            } => {
                self.workspace_tree.upsert(AgentNode {
                    agent_id,
                    stage,
                    target,
                    transport,
                    status: AgentStatus::Running,
                });
            }
            AgentEvent::AgentComplete {
                agent_id,
                confidence_grade,
                tokens_in,
                tokens_out,
                provider,
                ..
            } => {
                self.token_totals.record(&provider, tokens_in, tokens_out);
                self.workspace_tree.set_status(
                    &agent_id,
                    AgentStatus::Complete {
                        grade: confidence_grade,
                    },
                );
            }
            AgentEvent::HardFail {
                agent_id,
                error_kind,
                ..
            } => {
                self.workspace_tree
                    .set_status(&agent_id, AgentStatus::HardFailed { error_kind });
            }
            AgentEvent::CacheHit {
                agent_id, source, ..
            } => {
                self.workspace_tree
                    .set_status(&agent_id, AgentStatus::CacheHit { source });
            }
            AgentEvent::ToolCall { .. }
            | AgentEvent::ToolResult { .. }
            | AgentEvent::AuditFire { .. }
            | AgentEvent::AuditVerdict { .. }
            | AgentEvent::AuditDegraded { .. } => {
                // No state-shape effect in PR-6's projection — kept
                // out of the snapshot. PR-7 may wire audit verdicts
                // into the tree-view rendering.
            }
            AgentEvent::RuntimeComplete => {
                self.runtime_complete = true;
            }
        }
    }

    /// Note a `RecvError::Lagged(n)` observation. Surfaces in the
    /// snapshot under `lag` so the test harness can confirm
    /// backpressure-awareness rather than silent drop (per the
    /// `events.rs` module-level invariant).
    pub fn note_lag(&mut self, n: u64) {
        self.lag = self.lag.saturating_add(n);
    }

    /// Produce a serde-serialisable snapshot of state for diff against
    /// a replay-run snapshot. The snapshot omits the `Instant`-backed
    /// `last_activity` field of [`StuckDetector`] (rendering-only state
    /// that would diverge between live and replay).
    pub fn snapshot(&self) -> TuiSnapshot {
        TuiSnapshot {
            workspace_tree: self.workspace_tree.clone(),
            token_totals: self.token_totals.clone(),
            iteration: self.iteration,
            last_prior_model_sha: self.last_prior_model_sha.clone(),
            prev_prior_model_sha: self.prev_prior_model_sha.clone(),
            stuck_threshold_secs: self.stuck.threshold.as_secs(),
            lag: self.lag,
            runtime_complete: self.runtime_complete,
        }
    }
}

/// Serde-serialisable snapshot of [`TuiState`]. Compared byte-equal
/// between (live run → snapshot) and (replay → snapshot) in the
/// PR-6 replay test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiSnapshot {
    pub workspace_tree: Tree,
    pub token_totals: TokenTotals,
    pub iteration: u32,
    pub last_prior_model_sha: Option<String>,
    pub prev_prior_model_sha: Option<String>,
    pub stuck_threshold_secs: u64,
    pub lag: u64,
    pub runtime_complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_agents::events::CacheHitSource;

    fn agent_start(id: &str, stage: &str) -> AgentEvent {
        AgentEvent::AgentStart {
            agent_id: id.into(),
            parent_id: None,
            stage: stage.into(),
            target: "tgt".into(),
            fingerprint: "fp".into(),
            started_at: "t0".into(),
            transport: TransportFlavour::ClaudeCode,
        }
    }

    fn agent_complete(id: &str, tokens_in: u64, tokens_out: u64, provider: &str) -> AgentEvent {
        AgentEvent::AgentComplete {
            agent_id: id.into(),
            output_sha: "sha".into(),
            confidence_grade: Grade::Strong,
            tokens_in,
            tokens_out,
            ms: 0,
            provider: provider.into(),
        }
    }

    #[test]
    fn apply_agent_start_creates_running_node() {
        let mut s = TuiState::default();
        s.apply(agent_start("a1", "L3"));
        let snap = s.snapshot();
        let bucket = snap.workspace_tree.by_stage.get("L3").unwrap();
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].agent_id, "a1");
        assert!(matches!(bucket[0].status, AgentStatus::Running));
    }

    #[test]
    fn apply_agent_complete_transitions_to_complete_and_records_tokens() {
        let mut s = TuiState::default();
        s.apply(agent_start("a1", "L3"));
        s.apply(agent_complete("a1", 10, 5, "Anthropic"));
        let snap = s.snapshot();
        assert_eq!(
            snap.workspace_tree.by_stage["L3"][0].status,
            AgentStatus::Complete {
                grade: Grade::Strong
            }
        );
        assert_eq!(snap.token_totals.total_in(), 10);
        assert_eq!(snap.token_totals.total_out(), 5);
    }

    #[test]
    fn cache_hit_event_marks_node_cache_hit() {
        let mut s = TuiState::default();
        s.apply(agent_start("a1", "L3"));
        s.apply(AgentEvent::CacheHit {
            agent_id: "a1".into(),
            fingerprint: "fp".into(),
            replayed_at: "t".into(),
            source: CacheHitSource::AgentCache,
        });
        let snap = s.snapshot();
        assert_eq!(
            snap.workspace_tree.by_stage["L3"][0].status,
            AgentStatus::CacheHit {
                source: CacheHitSource::AgentCache
            }
        );
    }

    #[test]
    fn iteration_boundary_updates_iteration_and_rotates_priors() {
        let mut s = TuiState::default();
        s.apply(AgentEvent::IterationBoundary {
            iter: 0,
            prior_model_sha: None,
        });
        s.apply(AgentEvent::IterationBoundary {
            iter: 1,
            prior_model_sha: Some("sha-iter-1".into()),
        });
        assert_eq!(s.iteration, 1);
        assert_eq!(s.last_prior_model_sha.as_deref(), Some("sha-iter-1"));
        assert_eq!(s.prev_prior_model_sha, None);
        s.apply(AgentEvent::IterationBoundary {
            iter: 2,
            prior_model_sha: Some("sha-iter-1".into()),
        });
        // Same prior twice ⇒ convergence; that determination is made
        // in the iteration-bar widget; here we just confirm rotation.
        assert_eq!(s.prev_prior_model_sha.as_deref(), Some("sha-iter-1"));
    }

    #[test]
    fn note_lag_accumulates() {
        let mut s = TuiState::default();
        s.note_lag(3);
        s.note_lag(2);
        assert_eq!(s.snapshot().lag, 5);
    }

    #[test]
    fn snapshot_round_trips_via_serde_byte_equal() {
        // Snapshots are the artefact the replay test compares; the
        // (live → snapshot → bytes → snapshot) and (replay → snapshot
        // → bytes → snapshot) lines must produce byte-equal output.
        let mut a = TuiState::default();
        let mut b = TuiState::default();
        let seq = [
            agent_start("a1", "L3"),
            agent_start("a2", "L3"),
            agent_complete("a1", 100, 50, "Anthropic"),
            agent_complete("a2", 200, 75, "OpenAi"),
            AgentEvent::RuntimeComplete,
        ];
        for ev in &seq {
            a.apply(ev.clone());
            b.apply(ev.clone());
        }
        let a_bytes = serde_json::to_vec(&a.snapshot()).unwrap();
        let b_bytes = serde_json::to_vec(&b.snapshot()).unwrap();
        assert_eq!(a_bytes, b_bytes);
        let round_trip: TuiSnapshot = serde_json::from_slice(&a_bytes).unwrap();
        assert_eq!(round_trip, b.snapshot());
    }

    #[test]
    fn stuck_detector_under_threshold_returns_none() {
        let s = StuckDetector::new();
        assert!(s.check(Instant::now()).is_none());
    }

    #[test]
    fn token_totals_aggregate_across_providers() {
        let mut tt = TokenTotals::default();
        tt.record("Anthropic", 100, 50);
        tt.record("OpenAi", 200, 75);
        tt.record("Anthropic", 1, 1);
        assert_eq!(tt.total_in(), 301);
        assert_eq!(tt.total_out(), 126);
        assert_eq!(tt.by_provider[&Provider::Anthropic].tokens_in, 101);
        assert_eq!(tt.by_provider[&Provider::OpenAi].tokens_in, 200);
    }
}
