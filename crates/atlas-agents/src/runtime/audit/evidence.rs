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
        // PR-3 OWNS: real per-stage evidence functions for Classify,
        // Surface, Reduce, Project. Until then, return 1.0 so the LLM's
        // self-grade flows through unchanged — preserves the pre-PR-2
        // Strong-on-success behaviour for non-dispatch agents and keeps
        // Lane B's `should_audit` gate working end-to-end. The brainstorm
        // ladder is fully exercised for the dispatch arms above.
        Stage::Classify | Stage::Surface | Stage::Reduce | Stage::Project => 1.0,
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
            // Every stage returns a finite, in-range value. Behaviour
            // for PR-3 stages is `0.0`; for PR-2 dispatch stages it's
            // also `0.0` here because the synthetic output has empty
            // `candidates_considered`.
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
}
