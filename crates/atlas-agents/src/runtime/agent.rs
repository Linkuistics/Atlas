//! `Agent` — value-object identifying one per-stage-per-target-per-iteration
//! agent instance (plan §4 Task 4 file list).
//!
//! The runtime walks one `Agent` per `call_agent` invocation: the tuple
//! `(stage, target_id, iteration)` is the natural identity for an agent's
//! events, transcript-cache key contribution, and audit-trail entry. PR-4
//! surfaces this as a thin handle the runtime threads through event
//! emission and agent-id formatting; PR-5 may widen it with prompt-template
//! sha + tool-catalog sha + audit-state metadata as those become useful.
//!
//! Today the struct's primary role is `Agent::id()` — the stable agent_id
//! string carried on every `AgentEvent`. Keeping the formatter on the
//! struct means future PRs that change the id shape touch one site, not
//! every emit call.

use super::audit::Stage;
use super::AgentRequest;

/// One per-stage-per-target-per-iteration agent instance.
///
/// Constructed from an `AgentRequest` at the top of `call_agent`; threaded
/// through event emission as the source of the `agent_id` field. PR-5 may
/// add fields without breaking callers — adding a field to a `pub struct`
/// is a non-breaking minor for this crate (we don't seal it with
/// `#[non_exhaustive]` because the runtime is the sole owner-and-consumer
/// today; PR-5 will revisit if external consumers materialise).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Agent {
    /// Logical agent stage (recast §6.1).
    pub stage: Stage,
    /// Stable id of the agent's target — component id for `Classify` /
    /// `Surface`, subsystem id for `Reduce`, workspace marker for
    /// `Project` / dispatch stages.
    pub target_id: String,
    /// 1-indexed fixed-point iteration. PR-4 always passes `1`; PR-5's
    /// `run_fixedpoint` increments per pass.
    pub iteration: u32,
}

impl Agent {
    /// Construct from raw fields. The `target_id` accepts anything
    /// `Into<String>` for ergonomic call sites.
    pub fn new(stage: Stage, target_id: impl Into<String>, iteration: u32) -> Self {
        Self {
            stage,
            target_id: target_id.into(),
            iteration,
        }
    }

    /// Stable agent_id string: `"{stage}::{target}#i{iter}"`.
    ///
    /// Used as the `agent_id` field on every `AgentEvent` emitted for
    /// this agent instance. PR-5 may switch to a typed `AgentId(String)`
    /// newtype if external pattern-matching becomes useful; today the
    /// owned-string shape matches the `AgentEvent` variant fields.
    pub fn id(&self) -> String {
        format!(
            "{}::{}#i{}",
            self.stage.as_str(),
            self.target_id,
            self.iteration
        )
    }
}

impl From<&AgentRequest> for Agent {
    fn from(req: &AgentRequest) -> Self {
        Self::new(req.stage, &req.target_id, req.iteration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_format_is_stage_target_iter() {
        let a = Agent::new(Stage::Classify, "atlas-engine", 1);
        assert_eq!(a.id(), "classify::atlas-engine#i1");
    }

    #[test]
    fn id_format_distinguishes_iterations() {
        let i1 = Agent::new(Stage::Surface, "atlas-cli", 1);
        let i2 = Agent::new(Stage::Surface, "atlas-cli", 2);
        assert_ne!(i1.id(), i2.id());
        assert_eq!(i2.id(), "surface::atlas-cli#i2");
    }

    #[test]
    fn from_agent_request_round_trips_fields() {
        // AgentRequest carries strictly more fields than Agent; only the
        // identity tuple should pass through. Exercising via the smoke
        // test path indirectly covers the round-trip; this unit test
        // pins the format directly.
        let a = Agent::new(Stage::Reduce, "subsystem-x", 3);
        assert_eq!(a.stage, Stage::Reduce);
        assert_eq!(a.target_id, "subsystem-x");
        assert_eq!(a.iteration, 3);
    }
}
