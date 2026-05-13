//! PR-2 evidence-floor acceptance tests (decision row 5).
//!
//! Lane A's two-layer validator clamps the LLM's self-claimed grade
//! to the maximum the deterministic transcript-derived evidence
//! score supports. These tests pin the dispatch-stage clamping
//! semantics:
//!
//! - Empty transcript + claimed `Strong` → clamped to `Declines`.
//! - All-manifests-read transcript + claimed `Strong` → preserved as
//!   `Strong`.
//! - Half-manifests-read transcript + claimed `Strong` → clamped to
//!   `Moderate`.
//! - Claimed `Moderate` with `Strong`-supporting evidence → preserved
//!   as `Moderate` (clamp is `min`, not "assign evidence-max").
//!
//! Symmetric assertions land for `DispatchComponent` so the two
//! dispatch stages share the same clamping behaviour.

use std::collections::HashSet;

use atlas_agents::events::Grade;
use atlas_agents::runtime::audit::evidence::grade_ceiling;
use atlas_agents::runtime::audit::lane_a::{lane_a_validate, AgentOutput, Stage};
use atlas_agents::runtime::Transcript;
use serde_json::json;

fn empty_candidates() -> HashSet<String> {
    HashSet::new()
}

fn synthetic_output_claiming(grade: &str, candidates: &[(&str, &str)]) -> AgentOutput {
    let candidates_value: Vec<serde_json::Value> = candidates
        .iter()
        .map(|(id, manifest)| json!({ "id": id, "primary_manifest_path": manifest }))
        .collect();
    AgentOutput::from_value(json!({
        "confidence_grade": grade,
        "candidates_considered": candidates_value,
    }))
}

fn transcript_reading(paths: &[&str]) -> Transcript {
    let mut t = Transcript::new();
    for p in paths {
        t.push_synthetic_tool_call("parse_cargo_toml", json!({ "path": p }), json!({}));
    }
    t
}

#[tokio::test]
async fn dispatch_subsystem_claims_strong_with_empty_transcript_clamps_to_declines() {
    let output = synthetic_output_claiming("strong", &[("a", "a/Cargo.toml")]);
    let transcript = Transcript::new();
    let clamped = lane_a_validate(
        &output,
        Stage::DispatchSubsystem,
        &empty_candidates(),
        &transcript,
    )
    .await
    .expect("schema layer must pass for a well-formed claim envelope");
    assert_eq!(clamped, Grade::Declines);
}

#[tokio::test]
async fn dispatch_subsystem_claims_strong_with_all_manifests_read_stays_strong() {
    let output =
        synthetic_output_claiming("strong", &[("a", "a/Cargo.toml"), ("b", "b/Cargo.toml")]);
    let transcript = transcript_reading(&["a/Cargo.toml", "b/Cargo.toml"]);
    let clamped = lane_a_validate(
        &output,
        Stage::DispatchSubsystem,
        &empty_candidates(),
        &transcript,
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Strong);
}

#[tokio::test]
async fn dispatch_subsystem_claims_strong_with_half_manifests_read_clamps_to_moderate() {
    let output =
        synthetic_output_claiming("strong", &[("a", "a/Cargo.toml"), ("b", "b/Cargo.toml")]);
    let transcript = transcript_reading(&["a/Cargo.toml"]); // 1 of 2 = 0.5
    let clamped = lane_a_validate(
        &output,
        Stage::DispatchSubsystem,
        &empty_candidates(),
        &transcript,
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}

#[tokio::test]
async fn dispatch_component_clamping_is_symmetric_with_subsystems() {
    // The dispatch-component stage shares the same evidence-ratio
    // shape; the only difference is which agent emitted the candidates
    // (subsystem-level vs component-level). Same clamping behaviour.
    let output =
        synthetic_output_claiming("strong", &[("a", "a/Cargo.toml"), ("b", "b/Cargo.toml")]);
    let transcript = transcript_reading(&["a/Cargo.toml", "b/Cargo.toml"]);
    let clamped = lane_a_validate(
        &output,
        Stage::DispatchComponent,
        &empty_candidates(),
        &transcript,
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Strong);

    let transcript_empty = Transcript::new();
    let clamped_empty = lane_a_validate(
        &output,
        Stage::DispatchComponent,
        &empty_candidates(),
        &transcript_empty,
    )
    .await
    .unwrap();
    assert_eq!(clamped_empty, Grade::Declines);
}

#[test]
fn grade_ceiling_threshold_ladder() {
    assert_eq!(grade_ceiling(0.95), Grade::Strong);
    assert_eq!(grade_ceiling(0.90), Grade::Strong);
    assert_eq!(grade_ceiling(0.89), Grade::Moderate);
    assert_eq!(grade_ceiling(0.50), Grade::Moderate);
    assert_eq!(grade_ceiling(0.49), Grade::Weak);
    assert_eq!(grade_ceiling(0.10), Grade::Weak);
    assert_eq!(grade_ceiling(0.09), Grade::Declines);
    assert_eq!(grade_ceiling(0.00), Grade::Declines);
}

#[tokio::test]
async fn llm_may_grade_lower_than_evidence_max_but_never_higher() {
    // Evidence ceiling = Strong (1.0), claimed = Moderate; the clamp
    // is `min` so the LLM's lower self-grade wins. Tests that an LLM
    // legitimately self-grading conservatively isn't pushed up by
    // strong transcript evidence.
    let output = synthetic_output_claiming("moderate", &[("a", "a/Cargo.toml")]);
    let transcript = transcript_reading(&["a/Cargo.toml"]);
    let clamped = lane_a_validate(
        &output,
        Stage::DispatchSubsystem,
        &empty_candidates(),
        &transcript,
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}

#[tokio::test]
async fn dispatch_subsystem_missing_grade_field_defaults_to_strong_then_clamps() {
    // No `confidence_grade` field → `AgentOutput::confidence_grade()`
    // returns the default `Grade::Strong`. With no transcript reads
    // the evidence ceiling is `Declines`, so the clamp resolves to
    // `Declines`. This is the load-bearing fail-loud behaviour: an
    // LLM that omits the confidence field doesn't get a free pass.
    let output = AgentOutput::from_value(json!({
        "candidates_considered": [
            { "id": "a", "primary_manifest_path": "a/Cargo.toml" }
        ]
    }));
    let transcript = Transcript::new();
    let clamped = lane_a_validate(
        &output,
        Stage::DispatchSubsystem,
        &empty_candidates(),
        &transcript,
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Declines);
}
