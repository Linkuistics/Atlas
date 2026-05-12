//! Audit lanes for the agent runtime (recast §4.3).
//!
//! Lane A — **schema validation** — always fires on every agent output.
//! It rejects responses whose declared edge kinds do not resolve in
//! `component-ontology`, whose declared component ids do not resolve
//! in the candidate set, and (for stages that require surfaces)
//! responses that emit zero surfaces. Schema-fail incurs exactly one
//! retry; a second fail is a hard fail.
//!
//! Lane B — **cross-provider audit** — fires only when the producer's
//! `confidence_grade` is `Weak` or `Declines`. Pairs Anthropic-produced
//! output with an OpenAI auditor and vice-versa. Single-provider
//! configs fall back to a same-model auditor and emit
//! [`crate::events::AgentEvent::AuditDegraded`]. PR-5 ships the audit
//! decision + event emission scaffold; the actual auditor prompt is a
//! placeholder pending PR-7 wiring.

pub mod lane_a;
pub mod lane_b;

pub use lane_a::{lane_a_validate, requires_at_least_one_surface, AgentOutput, SchemaError, Stage};
pub use lane_b::{
    auditor_provider_for, lane_b_audit, provider_label, select_auditor_backend, should_audit,
    AuditVerdict, AuditorChoice,
};
