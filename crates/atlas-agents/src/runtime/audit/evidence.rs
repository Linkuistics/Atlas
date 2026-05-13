//! Per-stage deterministic evidence scoring.
//!
//! Lane A's two-layer validator calls [`compute_evidence_score`] +
//! [`grade_ceiling`] to clamp the LLM's self-grade to a maximum the
//! transcript-derived evidence supports. The dispatcher matches on
//! [`Stage`] and routes to the right per-stage scoring function.
//!
//! **PR-2 scope** ships scoring for the two dispatch stages
//! (`DispatchSubsystem` + `DispatchComponent`). The four remaining
//! stages (`Classify`, `Surface`, `Reduce`, `Project`) return `0.0`
//! until PR-3 adds their per-stage scoring functions — which clamps
//! their LLM-claimed grade to `Declines`. That fail-loud fallback
//! shows up immediately if a classify / surface / reduce / project
//! agent path somehow reaches the two-layer validator before PR-3
//! lands; brainstorm decision row 3 (per-stage evidence) is staged
//! incrementally across PR-2 + PR-3 by design.
//!
//! Score range is `[0.0, 1.0]` clamped to that interval; per-stage
//! functions are responsible for keeping their outputs inside the
//! range (the [`grade_ceiling`] thresholds are coupled to it).

use crate::events::Grade;
use crate::runtime::audit::lane_a::{AgentOutput, Stage};
use crate::runtime::Transcript;

/// Compute the deterministic evidence score for `output` against
/// `transcript`, dispatched by `stage`.
///
/// Returns a value in `[0.0, 1.0]`. The caller clamps the LLM's
/// claimed grade to [`grade_ceiling`] of this score.
pub fn compute_evidence_score(stage: Stage, transcript: &Transcript, output: &AgentOutput) -> f32 {
    match stage {
        Stage::DispatchSubsystem => dispatch_subsystems_evidence(transcript, output),
        Stage::DispatchComponent => dispatch_components_evidence(transcript, output),
        Stage::Classify => classify_evidence(transcript, output),
        Stage::Surface => surface_evidence(transcript, output),
        Stage::Reduce => reduce_evidence(transcript, output),
        Stage::Project => project_evidence(transcript, output),
    }
}

/// Map an evidence score in `[0.0, 1.0]` to the maximum [`Grade`] the
/// LLM's self-grade may be clamped to. Thresholds (decision row 5):
///
/// | score range  | ceiling      |
/// |--------------|--------------|
/// | `>= 0.9`     | `Strong`     |
/// | `>= 0.5`     | `Moderate`   |
/// | `>= 0.1`     | `Weak`       |
/// | `<  0.1`     | `Declines`   |
///
/// The ladder is symmetric with the four-grade rubric the dispatch
/// prompts advertise; an LLM that grades itself `Strong` with no
/// supporting reads is clamped to `Declines`, an evidence-grounded
/// `Strong` is preserved.
pub fn grade_ceiling(score: f32) -> Grade {
    if score >= 0.9 {
        Grade::Strong
    } else if score >= 0.5 {
        Grade::Moderate
    } else if score >= 0.1 {
        Grade::Weak
    } else {
        Grade::Declines
    }
}

/// Dispatch-subsystem evidence: ratio of L1 candidate manifest paths
/// the agent's tool calls touched (any tool call whose `args.path`
/// matched the candidate's `primary_manifest_path`) to total L1
/// candidates the agent reported considering. Empty candidate set →
/// `0.0` (worst case; the agent had nothing to claim against).
fn dispatch_subsystems_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let candidates = output.l1_candidates_referenced();
    if candidates.is_empty() {
        return 0.0;
    }
    let reads = transcript.read_file_paths();
    let manifests_read = candidates
        .iter()
        .filter(|c| reads.contains(&c.primary_manifest_path))
        .count();
    manifests_read as f32 / candidates.len() as f32
}

/// Dispatch-component evidence: same shape as
/// [`dispatch_subsystems_evidence`], scoped to a single subsystem's
/// component candidates. The output envelope reuses the
/// `candidates_considered` field shape — the dispatch-component agent
/// is told to populate it with the component-level candidates it
/// inspected.
fn dispatch_components_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let candidates = output.subsystem_component_candidates();
    if candidates.is_empty() {
        return 0.0;
    }
    let reads = transcript.read_file_paths();
    let manifests_read = candidates
        .iter()
        .filter(|c| reads.contains(&c.primary_manifest_path))
        .count();
    manifests_read as f32 / candidates.len() as f32
}

/// PR-3: per-component Classify evidence (decision row 5; brainstorm
/// §6.1).
///
/// Ladder:
///
/// | reads observed                               | score |
/// |----------------------------------------------|-------|
/// | manifest + entrypoint + classify tool called | 1.0   |
/// | manifest + classify tool called              | 0.6   |
/// | manifest only                                | 0.4   |
/// | none of the above                            | 0.0   |
///
/// The primary-manifest and source-entry-point paths are read from
/// `evidence_pointers[0]` / `evidence_pointers[1]` per the classify
/// prompt rubric's ordering convention. The expected classifier tool
/// is derived from the agent's declared `kind`
/// ([`AgentOutput::expected_classify_tool_id`]).
fn classify_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let reads = transcript.read_file_paths();
    let manifest_read = match output.primary_manifest_path() {
        Some(p) => reads.contains(&p),
        None => false,
    };
    let entrypoint_read = output
        .declared_entrypoint_path()
        .map(|p| reads.contains(&p))
        .unwrap_or(false);
    let classify_tool_called = transcript.tool_called(&output.expected_classify_tool_id());
    if manifest_read && entrypoint_read && classify_tool_called {
        1.0
    } else if manifest_read && classify_tool_called {
        0.6
    } else if manifest_read {
        0.4
    } else {
        0.0
    }
}

/// PR-3: per-component Surface evidence.
///
/// Ratio of inspected public-item sources to declared count. A
/// component with zero declared public items is vacuously satisfied
/// (returns 1.0) — surface extraction for a header-only or pure-data
/// component legitimately produces no surfaces.
///
/// An "inspection" is counted via either:
///
/// 1. A call to the `find_pub_items` tool (one per call), or
/// 2. A tool call whose `args.path` matches a declared surface's
///    `source_path`.
///
/// The sum is clamped to `1.0` so heavy over-inspection doesn't
/// inflate the score beyond Strong.
fn surface_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let declared = output.declared_public_items_count();
    if declared == 0 {
        return 1.0;
    }
    let declared_paths = output.declared_public_item_paths();
    let reads = transcript.read_file_paths();
    let path_intersections = reads.iter().filter(|p| declared_paths.contains(*p)).count();
    let pub_items_calls = transcript.tool_calls_for("find_pub_items").count();
    let inspected = path_intersections + pub_items_calls;
    (inspected as f32 / declared as f32).min(1.0)
}

/// PR-3: per-subsystem Reduce evidence.
///
/// Ratio of children the reducer accounted for to children the
/// runtime handed it. Empty child list → vacuously 1.0 (a subsystem
/// with no children is trivially reduced).
///
/// The reduce prompt rubric tells the reducer to echo the
/// per-subsystem child list back as `declared_child_component_ids`
/// (the denominator); the reducer's `component_ids` is what it
/// actually addressed (the numerator).
fn reduce_evidence(_transcript: &Transcript, output: &AgentOutput) -> f32 {
    let expected = output.declared_child_component_ids().len();
    if expected == 0 {
        return 1.0;
    }
    let observed = output.component_ids().len();
    (observed as f32 / expected as f32).min(1.0)
}

/// PR-3: workspace-level Project evidence.
///
/// Same shape as reduce, scoped to subsystems. Empty subsystem list →
/// vacuously 1.0 (a workspace with zero subsystems is trivially
/// projected).
fn project_evidence(_transcript: &Transcript, output: &AgentOutput) -> f32 {
    let expected = output.declared_subsystem_ids().len();
    if expected == 0 {
        return 1.0;
    }
    let observed = output.subsystem_catalog().len();
    (observed as f32 / expected as f32).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::audit::lane_a::L1CandidateRef;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn grade_ceiling_strong_at_or_above_0_9() {
        assert_eq!(grade_ceiling(1.0), Grade::Strong);
        assert_eq!(grade_ceiling(0.95), Grade::Strong);
        assert_eq!(grade_ceiling(0.90), Grade::Strong);
    }

    #[test]
    fn grade_ceiling_moderate_in_0_5_to_below_0_9() {
        assert_eq!(grade_ceiling(0.89), Grade::Moderate);
        assert_eq!(grade_ceiling(0.50), Grade::Moderate);
    }

    #[test]
    fn grade_ceiling_weak_in_0_1_to_below_0_5() {
        assert_eq!(grade_ceiling(0.49), Grade::Weak);
        assert_eq!(grade_ceiling(0.10), Grade::Weak);
    }

    #[test]
    fn grade_ceiling_declines_below_0_1() {
        assert_eq!(grade_ceiling(0.09), Grade::Declines);
        assert_eq!(grade_ceiling(0.0), Grade::Declines);
    }

    #[test]
    fn dispatch_subsystems_evidence_zero_when_no_candidates() {
        let transcript = Transcript::new();
        let output = AgentOutput::from_value(json!({}));
        assert_eq!(dispatch_subsystems_evidence(&transcript, &output), 0.0);
    }

    #[test]
    fn dispatch_subsystems_evidence_one_when_all_manifests_read() {
        let output = AgentOutput::from_value(json!({
            "candidates_considered": [
                { "id": "atlas-cli", "primary_manifest_path": "crates/atlas-cli/Cargo.toml" },
                { "id": "atlas-engine", "primary_manifest_path": "crates/atlas-engine/Cargo.toml" }
            ]
        }));
        let mut transcript = Transcript::new();
        transcript.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "crates/atlas-cli/Cargo.toml" }),
            json!({}),
        );
        transcript.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "crates/atlas-engine/Cargo.toml" }),
            json!({}),
        );
        let score = dispatch_subsystems_evidence(&transcript, &output);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "all-manifests-read must score 1.0, got {score}"
        );
    }

    #[test]
    fn dispatch_subsystems_evidence_half_when_half_manifests_read() {
        let output = AgentOutput::from_value(json!({
            "candidates_considered": [
                { "id": "a", "primary_manifest_path": "a/Cargo.toml" },
                { "id": "b", "primary_manifest_path": "b/Cargo.toml" }
            ]
        }));
        let mut transcript = Transcript::new();
        transcript.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "a/Cargo.toml" }),
            json!({}),
        );
        let score = dispatch_subsystems_evidence(&transcript, &output);
        assert!(
            (score - 0.5).abs() < f32::EPSILON,
            "half-manifests-read must score 0.5, got {score}"
        );
    }

    #[test]
    fn dispatch_subsystems_evidence_zero_when_empty_transcript() {
        let output = AgentOutput::from_value(json!({
            "candidates_considered": [
                { "id": "a", "primary_manifest_path": "a/Cargo.toml" }
            ]
        }));
        let transcript = Transcript::new();
        assert_eq!(dispatch_subsystems_evidence(&transcript, &output), 0.0);
    }

    #[test]
    fn compute_evidence_score_dispatches_by_stage() {
        // Empty-output baseline: dispatch arms return 0.0 (no candidates
        // claimed); Classify/Surface returns 0.0 (no evidence); Reduce
        // and Project return 1.0 (vacuously satisfied — zero declared
        // children/subsystems). All values must lie in [0, 1].
        let output = AgentOutput::from_value(json!({}));
        let transcript = Transcript::new();
        for stage in [
            Stage::DispatchSubsystem,
            Stage::DispatchComponent,
            Stage::Classify,
            Stage::Surface,
            Stage::Reduce,
            Stage::Project,
        ] {
            let s = compute_evidence_score(stage, &transcript, &output);
            assert!(
                (0.0..=1.0).contains(&s),
                "stage {stage:?} score out of range: {s}"
            );
        }
    }

    #[test]
    fn l1_candidate_ref_round_trips_via_serde_json() {
        let original = L1CandidateRef {
            id: "atlas-cli".to_string(),
            primary_manifest_path: PathBuf::from("crates/atlas-cli/Cargo.toml"),
        };
        let v = serde_json::to_value(&original).unwrap();
        let back: L1CandidateRef = serde_json::from_value(v).unwrap();
        assert_eq!(back, original);
    }

    // ---- PR-3 per-stage evidence unit tests ---------------------------

    #[test]
    fn classify_evidence_strong_when_manifest_entrypoint_and_tool_called() {
        let output = AgentOutput::from_value(json!({
            "kind": "rust-library",
            "evidence_pointers": [
                { "path": "crates/atlas-cli/Cargo.toml" },
                { "path": "crates/atlas-cli/src/main.rs" }
            ]
        }));
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "crates/atlas-cli/Cargo.toml" }),
            json!({}),
        );
        t.push_synthetic_tool_call(
            "read_file",
            json!({ "path": "crates/atlas-cli/src/main.rs" }),
            json!({}),
        );
        let score = classify_evidence(&t, &output);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "manifest+entrypoint+tool must score 1.0, got {score}"
        );
    }

    #[test]
    fn classify_evidence_moderate_when_manifest_and_tool_only() {
        let output = AgentOutput::from_value(json!({
            "kind": "rust-library",
            "evidence_pointers": [
                { "path": "crates/atlas-cli/Cargo.toml" },
                { "path": "crates/atlas-cli/src/main.rs" }
            ]
        }));
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "crates/atlas-cli/Cargo.toml" }),
            json!({}),
        );
        // entrypoint NOT read.
        let score = classify_evidence(&t, &output);
        assert!(
            (score - 0.6).abs() < f32::EPSILON,
            "manifest+tool no entrypoint must score 0.6, got {score}"
        );
    }

    #[test]
    fn classify_evidence_weak_when_manifest_only_no_tool() {
        let output = AgentOutput::from_value(json!({
            "kind": "rust-library",
            "evidence_pointers": [
                { "path": "crates/atlas-cli/Cargo.toml" }
            ]
        }));
        let mut t = Transcript::new();
        // Tool call that LANDS the manifest path but isn't the
        // classifier tool (just a generic read).
        t.push_synthetic_tool_call(
            "read_file",
            json!({ "path": "crates/atlas-cli/Cargo.toml" }),
            json!({}),
        );
        let score = classify_evidence(&t, &output);
        assert!(
            (score - 0.4).abs() < f32::EPSILON,
            "manifest-read-only must score 0.4, got {score}"
        );
    }

    #[test]
    fn classify_evidence_zero_when_no_reads() {
        let output = AgentOutput::from_value(json!({
            "kind": "rust-library",
            "evidence_pointers": [
                { "path": "Cargo.toml" }
            ]
        }));
        let t = Transcript::new();
        let score = classify_evidence(&t, &output);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn surface_evidence_one_when_zero_declared() {
        let output = AgentOutput::from_value(json!({ "surfaces": [] }));
        let t = Transcript::new();
        assert!((surface_evidence(&t, &output) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn surface_evidence_full_coverage_when_paths_read() {
        let output = AgentOutput::from_value(json!({
            "surfaces": [
                { "source_path": "crates/a/src/lib.rs" },
                { "source_path": "crates/b/src/lib.rs" }
            ]
        }));
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "read_file",
            json!({ "path": "crates/a/src/lib.rs" }),
            json!({}),
        );
        t.push_synthetic_tool_call(
            "read_file",
            json!({ "path": "crates/b/src/lib.rs" }),
            json!({}),
        );
        assert!((surface_evidence(&t, &output) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn surface_evidence_partial_when_some_paths_unread() {
        let output = AgentOutput::from_value(json!({
            "surfaces": [
                { "source_path": "a.rs" },
                { "source_path": "b.rs" }
            ]
        }));
        let mut t = Transcript::new();
        t.push_synthetic_tool_call("read_file", json!({ "path": "a.rs" }), json!({}));
        let score = surface_evidence(&t, &output);
        assert!((score - 0.5).abs() < f32::EPSILON, "got {score}");
    }

    #[test]
    fn reduce_evidence_vacuous_when_no_declared_children() {
        // declared_child_component_ids absent → expected = 0 → 1.0.
        let output = AgentOutput::from_value(json!({}));
        let t = Transcript::new();
        assert!((reduce_evidence(&t, &output) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn reduce_evidence_full_coverage() {
        let output = AgentOutput::from_value(json!({
            "declared_child_component_ids": ["a", "b", "c"],
            "component_ids": ["a", "b", "c"]
        }));
        let t = Transcript::new();
        assert!((reduce_evidence(&t, &output) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn reduce_evidence_partial_coverage() {
        let output = AgentOutput::from_value(json!({
            "declared_child_component_ids": ["a", "b", "c", "d"],
            "component_ids": ["a", "b"]
        }));
        let t = Transcript::new();
        let score = reduce_evidence(&t, &output);
        assert!((score - 0.5).abs() < f32::EPSILON, "got {score}");
    }

    #[test]
    fn project_evidence_vacuous_when_no_declared_subsystems() {
        let output = AgentOutput::from_value(json!({}));
        let t = Transcript::new();
        assert!((project_evidence(&t, &output) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn project_evidence_full_coverage() {
        let output = AgentOutput::from_value(json!({
            "declared_subsystem_ids": ["agents", "cli"],
            "subsystem_catalog": [
                { "subsystem_id": "agents", "purpose": "x", "component_count": 1 },
                { "subsystem_id": "cli", "purpose": "y", "component_count": 1 }
            ]
        }));
        let t = Transcript::new();
        assert!((project_evidence(&t, &output) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn project_evidence_partial_coverage() {
        let output = AgentOutput::from_value(json!({
            "declared_subsystem_ids": ["a", "b", "c", "d"],
            "subsystem_catalog": [
                { "subsystem_id": "a", "purpose": "x", "component_count": 1 }
            ]
        }));
        let t = Transcript::new();
        let score = project_evidence(&t, &output);
        assert!((score - 0.25).abs() < f32::EPSILON, "got {score}");
    }
}
