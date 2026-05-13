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
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use component_ontology::EdgeKind;

use crate::events::Grade;

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

/// One L1 candidate the dispatch agent considered. Emitted by the
/// LLM as part of its dispatch-stage YAML envelope under the
/// `candidates_considered:` field; Lane A's evidence-floor scorer
/// reads this against the transcript's `read_file_paths()` to compute
/// the dispatch-stage evidence ratio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct L1CandidateRef {
    /// Stable id of the candidate (e.g. `"atlas-cli"`).
    pub id: String,
    /// Primary manifest path the agent should have read to ground its
    /// dispatch decision (e.g. `crates/atlas-cli/Cargo.toml`). Lane A
    /// matches this against the transcript's file-read set.
    pub primary_manifest_path: PathBuf,
}

/// Structured agent output the runtime hands to Lane A. The
/// `value` field is the raw JSON the model emitted; the optional
/// helper-parsed projections are populated by `tool_loop_*` parsers
/// for stages where the wire shape is well-known.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentOutput {
    /// Raw JSON value emitted by the model. Lane A inspects this
    /// directly; downstream callers may extract typed views.
    pub value: Value,
    /// PR-2: raw concatenation of the LLM's `content[].text` blocks.
    /// The dispatch-stage YAML-migration path fence-extracts this via
    /// [`crate::runtime::prompt_examples::extract_yaml_fence`] and
    /// hands the body to `serde_yaml::from_str`. Empty when the
    /// response arrived in a shape that did not carry text blocks
    /// (e.g. the `response.output` envelope used by the test backend).
    #[serde(default)]
    pub text: String,
}

impl AgentOutput {
    /// Construct from a raw JSON value (no text-block content).
    pub fn from_value(value: Value) -> Self {
        Self {
            value,
            text: String::new(),
        }
    }

    /// Construct from a raw JSON value + the originating LLM text
    /// content. PR-2: `parse_final_output` populates both fields when
    /// the response carries text blocks.
    pub fn from_value_and_text(value: Value, text: String) -> Self {
        Self { value, text }
    }

    /// LLM's self-claimed confidence grade. Reads `value["confidence_grade"]`
    /// as a case-insensitive string and maps to [`Grade`]. Returns
    /// `Grade::Strong` if the field is absent — preserves the pre-PR-2
    /// hardcoded behaviour for backends that don't emit a grade.
    pub fn confidence_grade(&self) -> Grade {
        self.value
            .get("confidence_grade")
            .and_then(Value::as_str)
            .and_then(|s| match s.to_lowercase().as_str() {
                "strong" => Some(Grade::Strong),
                "moderate" => Some(Grade::Moderate),
                "weak" => Some(Grade::Weak),
                "declines" => Some(Grade::Declines),
                _ => None,
            })
            .unwrap_or(Grade::Strong)
    }

    /// L1 candidates the dispatch-subsystem agent emitted in its output
    /// envelope. Used by [`crate::runtime::audit::evidence`]
    /// `dispatch_subsystems_evidence` to compute the
    /// reads-vs-candidates ratio. Returns an empty `Vec` when the
    /// `candidates_considered` field is absent or malformed.
    pub fn l1_candidates_referenced(&self) -> Vec<L1CandidateRef> {
        self.value
            .get("candidates_considered")
            .and_then(|v| serde_json::from_value::<Vec<L1CandidateRef>>(v.clone()).ok())
            .unwrap_or_default()
    }

    /// L1 candidates the dispatch-component agent emitted in its output
    /// envelope. Same envelope shape as
    /// [`Self::l1_candidates_referenced`], scoped to one subsystem's
    /// component candidates.
    pub fn subsystem_component_candidates(&self) -> Vec<L1CandidateRef> {
        self.l1_candidates_referenced()
    }
}

/// True iff the stage requires `len(output.surfaces) >= 1`. Pure helper
/// so `lane_a_validate` and the retry harness agree on the predicate.
pub fn requires_at_least_one_surface(stage: Stage) -> bool {
    matches!(stage, Stage::Surface)
}

/// Run Lane A on `output` against the per-call `candidate_ids` set
/// + `transcript`.
///
/// **PR-2: two-layer validation.**
///
/// - **Layer 1 (schema)** — the pre-existing structural checks
///   (unknown edge kinds, unknown component ids, missing surfaces on
///   `Stage::Surface`). Fails fast on the first violation, surfaced
///   via [`SchemaError`].
/// - **Layer 2 (evidence floor)** — once Layer 1 passes, the LLM's
///   self-claimed grade ([`AgentOutput::confidence_grade`]) is
///   clamped against the deterministic evidence score
///   ([`crate::runtime::audit::evidence::compute_evidence_score`])
///   via [`crate::runtime::audit::evidence::grade_ceiling`]. The
///   clamped grade is returned to the caller, which propagates it
///   through `AgentComplete` events + transcript-frame metadata.
///
/// `candidate_ids` is the set of component ids the dispatcher already
/// resolved (PR-4: from override files; PR-5 widens to LLM dispatch).
/// An empty set means "Lane A skips the component-id check for this
/// call" — used by the dispatch stages themselves, which decide the
/// candidate set rather than consult one. The dispatch-stage call
/// sites in [`crate::runtime::dispatch`] pass an empty `HashSet`.
pub async fn lane_a_validate(
    output: &AgentOutput,
    stage: Stage,
    candidate_ids: &HashSet<String>,
    transcript: &crate::runtime::Transcript,
) -> Result<Grade, SchemaError> {
    // Layer 1: schema validation (pre-PR-2 behaviour preserved).
    schema_validate(output, stage, candidate_ids)?;

    // Layer 2: deterministic evidence-floor clamp.
    let claimed = output.confidence_grade();
    let evidence_score =
        crate::runtime::audit::evidence::compute_evidence_score(stage, transcript, output);
    let evidence_max = crate::runtime::audit::evidence::grade_ceiling(evidence_score);
    Ok(claimed.min(evidence_max))
}

/// Layer 1 schema validation, exposed as a free helper so dispatch
/// call sites can run it against the parsed override-file path (where
/// the evidence-floor doesn't apply because the synthetic transcript
/// is empty by construction).
fn schema_validate(
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
    use crate::runtime::Transcript;
    use serde_json::json;

    fn empty_candidates() -> HashSet<String> {
        HashSet::new()
    }

    fn empty_transcript() -> Transcript {
        Transcript::new()
    }

    #[tokio::test]
    async fn accepts_object_with_no_edges_or_components() {
        let out = AgentOutput::from_value(json!({}));
        // PR-2: Classify stage's evidence-floor fallback is 1.0 until
        // PR-3 lands the real classify-stage scorer; the LLM's
        // claimed grade (default Strong on absent field) flows through.
        let grade = lane_a_validate(
            &out,
            Stage::Classify,
            &empty_candidates(),
            &empty_transcript(),
        )
        .await
        .unwrap();
        assert_eq!(grade, Grade::Strong);
    }

    #[tokio::test]
    async fn rejects_non_object_root() {
        let out = AgentOutput::from_value(json!([1, 2, 3]));
        let err = lane_a_validate(
            &out,
            Stage::Classify,
            &empty_candidates(),
            &empty_transcript(),
        )
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
        let err = lane_a_validate(
            &out,
            Stage::Classify,
            &empty_candidates(),
            &empty_transcript(),
        )
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
        let _grade = lane_a_validate(
            &out,
            Stage::Classify,
            &empty_candidates(),
            &empty_transcript(),
        )
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
        let err = lane_a_validate(&out, Stage::Classify, &candidates, &empty_transcript())
            .await
            .unwrap_err();
        assert!(matches!(err, SchemaError::UnknownComponentId(ref id) if id == "stranger"));
    }

    #[tokio::test]
    async fn requires_at_least_one_surface_on_surface_stage() {
        let out = AgentOutput::from_value(json!({ "surfaces": [] }));
        let err = lane_a_validate(
            &out,
            Stage::Surface,
            &empty_candidates(),
            &empty_transcript(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SchemaError::NoSurfacesEmitted { .. }));
    }

    #[tokio::test]
    async fn surface_count_check_skipped_on_non_surface_stage() {
        // Classify stage tolerates absent / empty surfaces.
        let out = AgentOutput::from_value(json!({ "surfaces": [] }));
        let _grade = lane_a_validate(
            &out,
            Stage::Classify,
            &empty_candidates(),
            &empty_transcript(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn surface_stage_accepts_nonempty_surfaces() {
        let out = AgentOutput::from_value(json!({
            "surfaces": [{ "name": "GetWidget" }]
        }));
        let _grade = lane_a_validate(
            &out,
            Stage::Surface,
            &empty_candidates(),
            &empty_transcript(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dispatch_subsystem_with_all_manifests_read_returns_strong() {
        // Output emits `confidence_grade: strong` + 2 candidates; the
        // transcript records `parse_cargo_toml` calls for both manifest
        // paths. Evidence = 1.0 → ceiling = Strong; claimed = Strong;
        // clamped = Strong.
        let out = AgentOutput::from_value(json!({
            "confidence_grade": "strong",
            "candidates_considered": [
                { "id": "a", "primary_manifest_path": "a/Cargo.toml" },
                { "id": "b", "primary_manifest_path": "b/Cargo.toml" }
            ]
        }));
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "a/Cargo.toml" }),
            json!({}),
        );
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "b/Cargo.toml" }),
            json!({}),
        );
        let grade = lane_a_validate(&out, Stage::DispatchSubsystem, &empty_candidates(), &t)
            .await
            .unwrap();
        assert_eq!(grade, Grade::Strong);
    }

    #[tokio::test]
    async fn dispatch_subsystem_claims_strong_with_empty_transcript_clamps_to_declines() {
        let out = AgentOutput::from_value(json!({
            "confidence_grade": "strong",
            "candidates_considered": [
                { "id": "a", "primary_manifest_path": "a/Cargo.toml" }
            ]
        }));
        let grade = lane_a_validate(
            &out,
            Stage::DispatchSubsystem,
            &empty_candidates(),
            &empty_transcript(),
        )
        .await
        .unwrap();
        assert_eq!(grade, Grade::Declines);
    }

    #[tokio::test]
    async fn claimed_moderate_is_preserved_when_evidence_supports_strong() {
        // Evidence ceiling = Strong (1.0), claimed = Moderate; the
        // clamp is `min` so the LLM's lower self-grade wins.
        let out = AgentOutput::from_value(json!({
            "confidence_grade": "moderate",
            "candidates_considered": [
                { "id": "a", "primary_manifest_path": "a/Cargo.toml" }
            ]
        }));
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "a/Cargo.toml" }),
            json!({}),
        );
        let grade = lane_a_validate(&out, Stage::DispatchSubsystem, &empty_candidates(), &t)
            .await
            .unwrap();
        assert_eq!(grade, Grade::Moderate);
    }
}
