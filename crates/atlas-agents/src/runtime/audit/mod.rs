//! Audit lanes for the agent runtime (recast §4.3).
//!
//! Lane A — **schema validation** — always fires on every agent output.
//! It rejects responses whose declared edge kinds do not resolve in
//! `component-ontology`, whose declared component ids do not resolve
//! in the candidate set, and (for stages that require surfaces)
//! responses that emit zero surfaces. Schema-fail incurs exactly one
//! retry; a second fail is a hard fail.
//!
//! Lane B — cross-provider audit — lands in PR-5. Scaffolding is
//! deliberately omitted here; do not pre-create `lane_b.rs`.

pub mod lane_a;

pub use lane_a::{lane_a_validate, requires_at_least_one_surface, AgentOutput, SchemaError, Stage};
