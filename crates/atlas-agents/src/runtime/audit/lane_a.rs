//! Lane A — schema validation against ontology constraints (recast §4.3).
//!
//! Lane A is the cheap, always-on guard. It checks the LLM-emitted
//! `AgentOutput` for three classes of malformedness:
//!
//! 1. **Unknown edge kinds.** Every entry under `output.edges[i].kind`
//!    must parse via `component_ontology::EdgeKind::parse`. An edge
//!    kind the open ontology does not know is structurally invalid.
//! 2. **Unknown component ids.** Every entry under
//!    `output.edges[i].from` and `output.edges[i].to`, plus every
//!    `output.components[i].id`, must appear in the per-call candidate
//!    set the runtime hands to `lane_a_validate`. PR-4's candidate
//!    set comes from the override files; PR-5 widens this to the
//!    LLM-dispatched candidate set.
//! 3. **Missing surfaces.** Stages that promise to emit surfaces
//!    (currently `Stage::Surface`) must emit at least one entry under
//!    `output.surfaces`. An empty surface array on a Surface-stage
//!    response is a schema violation.
//!
//! Lane A is deliberately conservative: it rejects on *structurally
//! impossible* output, not on *semantically suspect* output. The
//! cross-provider Lane B (PR-5) handles the latter.
//!
//! # Retry semantics
//!
//! `call_agent` wraps `lane_a_validate` and performs exactly one
//! retry on `Err`. A second failure surfaces as `AgentError::LaneAFail`,
//! at which point the runtime emits `HardFail`. PR-4 implements the
//! retry harness in `crate::runtime::call_agent`; this file owns the
//! validation predicate only.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use component_ontology::EdgeKind;

/// Per-stage discriminator for Lane A schema validation. The stage
/// drives which sub-checks fire — e.g. only `Stage::Surface` requires
/// `len(surfaces) >= 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Workspace → subsystem partition. Reads the override file in PR-4.
    DispatchSubsystem,
    /// Subsystem → component partition. Reads the override file in PR-4.
    DispatchComponent,
    /// Per-component classification (kind + role + lifecycle).
    Classify,
    /// Per-component surface extraction.
    Surface,
    /// Per-subsystem reduce.
    Reduce,
    /// Workspace-level projection (L9).
    Project,
}

impl Stage {
    /// Stable wire form for logging / cache-key contribution. snake_case
    /// so it's filesystem-safe and survives serde round-trips.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DispatchSubsystem => "dispatch_subsystem",
            Self::DispatchComponent => "dispatch_component",
            Self::Classify => "classify",
            Self::Surface => "surface",
            Self::Reduce => "reduce",
            Self::Project => "project",
        }
    }
}

/// Lane A schema error. Surfaces the exact violation so the retry
/// prompt can mention what to fix.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("output is not a JSON object")]
    NotAnObject,
    #[error("unknown edge kind `{0}`")]
    UnknownEdgeKind(String),
    #[error("unknown component id `{0}` (not in candidate set)")]
    UnknownComponentId(String),
    #[error("stage {stage} requires at least one surface, got zero")]
    NoSurfacesEmitted { stage: &'static str },
    #[error("malformed edge entry: {0}")]
    MalformedEdge(String),
    #[error("malformed component entry: {0}")]
    MalformedComponent(String),
}

/// Structured agent output the runtime hands to Lane A. The
/// `value` field is the raw JSON the model emitted; the optional
/// helper-parsed projections are populated by `tool_loop_*` parsers
/// for stages where the wire shape is well-known.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutput {
    /// Raw JSON value emitted by the model. Lane A inspects this
    /// directly; downstream callers may extract typed views.
    pub value: Value,
}

impl AgentOutput {
    /// Construct from a raw JSON value.
    pub fn from_value(value: Value) -> Self {
        Self { value }
    }
}

/// True iff the stage requires `len(output.surfaces) >= 1`. Pure helper
/// so `lane_a_validate` and the retry harness agree on the predicate.
pub fn requires_at_least_one_surface(stage: Stage) -> bool {
    matches!(stage, Stage::Surface)
}

/// Run Lane A on `output` against the per-call `candidate_ids` set.
///
/// `candidate_ids` is the set of component ids the dispatcher already
/// resolved (PR-4: from override files; PR-5 widens to LLM dispatch).
/// An empty set means "Lane A skips the component-id check for this
/// call" — used by the dispatch stages themselves, which decide the
/// candidate set rather than consult one.
pub async fn lane_a_validate(
    output: &AgentOutput,
    stage: Stage,
    candidate_ids: &HashSet<String>,
) -> Result<(), SchemaError> {
    let Value::Object(map) = &output.value else {
        return Err(SchemaError::NotAnObject);
    };

    // 1. Edge-kind + edge-participant checks.
    if let Some(edges) = map.get("edges") {
        let edge_arr = edges.as_array().ok_or_else(|| {
            SchemaError::MalformedEdge("`edges` field is not an array".to_string())
        })?;
        for (idx, edge) in edge_arr.iter().enumerate() {
            let edge_obj = edge.as_object().ok_or_else(|| {
                SchemaError::MalformedEdge(format!("edges[{idx}] is not an object"))
            })?;
            let kind = edge_obj
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SchemaError::MalformedEdge(format!(
                        "edges[{idx}].kind is missing or not a string"
                    ))
                })?;
            if EdgeKind::parse(kind).is_none() {
                return Err(SchemaError::UnknownEdgeKind(kind.to_string()));
            }
            if !candidate_ids.is_empty() {
                for endpoint in ["from", "to"] {
                    let id = edge_obj
                        .get(endpoint)
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            SchemaError::MalformedEdge(format!(
                                "edges[{idx}].{endpoint} is missing or not a string"
                            ))
                        })?;
                    if !candidate_ids.contains(id) {
                        return Err(SchemaError::UnknownComponentId(id.to_string()));
                    }
                }
            }
        }
    }

    // 2. Component-id checks.
    if !candidate_ids.is_empty() {
        if let Some(components) = map.get("components") {
            let comp_arr = components.as_array().ok_or_else(|| {
                SchemaError::MalformedComponent("`components` field is not an array".to_string())
            })?;
            for (idx, comp) in comp_arr.iter().enumerate() {
                let comp_obj = comp.as_object().ok_or_else(|| {
                    SchemaError::MalformedComponent(format!("components[{idx}] is not an object"))
                })?;
                let id = comp_obj.get("id").and_then(Value::as_str).ok_or_else(|| {
                    SchemaError::MalformedComponent(format!(
                        "components[{idx}].id is missing or not a string"
                    ))
                })?;
                if !candidate_ids.contains(id) {
                    return Err(SchemaError::UnknownComponentId(id.to_string()));
                }
            }
        }
    }

    // 3. Surface-count check (stage-conditional).
    if requires_at_least_one_surface(stage) {
        let surfaces = map.get("surfaces").and_then(Value::as_array);
        let count = surfaces.map(|s| s.len()).unwrap_or(0);
        if count == 0 {
            return Err(SchemaError::NoSurfacesEmitted {
                stage: stage.as_str(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_candidates() -> HashSet<String> {
        HashSet::new()
    }

    #[tokio::test]
    async fn accepts_object_with_no_edges_or_components() {
        let out = AgentOutput::from_value(json!({}));
        lane_a_validate(&out, Stage::Classify, &empty_candidates())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rejects_non_object_root() {
        let out = AgentOutput::from_value(json!([1, 2, 3]));
        let err = lane_a_validate(&out, Stage::Classify, &empty_candidates())
            .await
            .unwrap_err();
        assert!(matches!(err, SchemaError::NotAnObject));
    }

    #[tokio::test]
    async fn rejects_unknown_edge_kind() {
        let out = AgentOutput::from_value(json!({
            "edges": [
                { "kind": "frobnicates", "from": "a", "to": "b" }
            ]
        }));
        let err = lane_a_validate(&out, Stage::Classify, &empty_candidates())
            .await
            .unwrap_err();
        assert!(matches!(err, SchemaError::UnknownEdgeKind(ref k) if k == "frobnicates"));
    }

    #[tokio::test]
    async fn accepts_known_edge_kind() {
        let out = AgentOutput::from_value(json!({
            "edges": [
                { "kind": "depends-on", "from": "a", "to": "b" }
            ]
        }));
        lane_a_validate(&out, Stage::Classify, &empty_candidates())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rejects_unknown_component_id() {
        let mut candidates = HashSet::new();
        candidates.insert("a".to_string());
        let out = AgentOutput::from_value(json!({
            "edges": [
                { "kind": "depends-on", "from": "a", "to": "stranger" }
            ]
        }));
        let err = lane_a_validate(&out, Stage::Classify, &candidates)
            .await
            .unwrap_err();
        assert!(matches!(err, SchemaError::UnknownComponentId(ref id) if id == "stranger"));
    }

    #[tokio::test]
    async fn requires_at_least_one_surface_on_surface_stage() {
        let out = AgentOutput::from_value(json!({ "surfaces": [] }));
        let err = lane_a_validate(&out, Stage::Surface, &empty_candidates())
            .await
            .unwrap_err();
        assert!(matches!(err, SchemaError::NoSurfacesEmitted { .. }));
    }

    #[tokio::test]
    async fn surface_count_check_skipped_on_non_surface_stage() {
        // Classify stage tolerates absent / empty surfaces.
        let out = AgentOutput::from_value(json!({ "surfaces": [] }));
        lane_a_validate(&out, Stage::Classify, &empty_candidates())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn surface_stage_accepts_nonempty_surfaces() {
        let out = AgentOutput::from_value(json!({
            "surfaces": [{ "name": "GetWidget" }]
        }));
        lane_a_validate(&out, Stage::Surface, &empty_candidates())
            .await
            .unwrap();
    }
}
