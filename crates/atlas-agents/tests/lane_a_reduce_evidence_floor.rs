//! PR-3 evidence-floor acceptance tests for the Reduce stage.
//!
//! Reduce evidence is the coverage ratio:
//! `len(component_ids) / len(declared_child_component_ids)`, clamped to
//! 1.0. Zero declared children → vacuously 1.0.

use std::collections::HashSet;

use atlas_agents::events::Grade;
use atlas_agents::runtime::audit::lane_a::{lane_a_validate, AgentOutput, Stage};
use atlas_agents::runtime::Transcript;
use serde_json::json;

fn empty_candidates() -> HashSet<String> {
    HashSet::new()
}

#[tokio::test]
async fn reduce_claims_strong_vacuously_when_no_declared_children() {
    // `declared_child_component_ids` empty → ratio = 1.0 → ceiling =
    // Strong → claimed Strong preserved. A subsystem with no children
    // is legitimately Strong-graded (the trivial-success case).
    let out = AgentOutput::from_value(json!({
        "declared_child_component_ids": [],
        "component_ids": [],
        "confidence_grade": "strong"
    }));
    let clamped = lane_a_validate(&out, Stage::Reduce, &empty_candidates(), &Transcript::new())
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Strong);
}

#[tokio::test]
async fn reduce_claims_strong_with_full_coverage_stays_strong() {
    let out = AgentOutput::from_value(json!({
        "declared_child_component_ids": ["a", "b", "c"],
        "component_ids": ["a", "b", "c"],
        "confidence_grade": "strong"
    }));
    let clamped = lane_a_validate(&out, Stage::Reduce, &empty_candidates(), &Transcript::new())
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Strong);
}

#[tokio::test]
async fn reduce_claims_strong_with_half_coverage_clamps_to_moderate() {
    // 2 of 4 children covered → ratio = 0.5 → ceiling = Moderate.
    let out = AgentOutput::from_value(json!({
        "declared_child_component_ids": ["a", "b", "c", "d"],
        "component_ids": ["a", "b"],
        "confidence_grade": "strong"
    }));
    let clamped = lane_a_validate(&out, Stage::Reduce, &empty_candidates(), &Transcript::new())
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}

#[tokio::test]
async fn reduce_claims_strong_with_zero_coverage_clamps_to_declines() {
    // declared children non-empty, component_ids empty → ratio = 0.0.
    let out = AgentOutput::from_value(json!({
        "declared_child_component_ids": ["a", "b"],
        "component_ids": [],
        "confidence_grade": "strong"
    }));
    let clamped = lane_a_validate(&out, Stage::Reduce, &empty_candidates(), &Transcript::new())
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Declines);
}

#[tokio::test]
async fn reduce_llm_may_claim_lower_than_evidence_max() {
    let out = AgentOutput::from_value(json!({
        "declared_child_component_ids": ["a", "b"],
        "component_ids": ["a", "b"],
        "confidence_grade": "moderate"
    }));
    let clamped = lane_a_validate(&out, Stage::Reduce, &empty_candidates(), &Transcript::new())
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}
