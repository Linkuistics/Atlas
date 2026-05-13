//! PR-3 evidence-floor acceptance tests for the Project stage.
//!
//! Same shape as reduce, scoped to subsystems:
//! `len(subsystem_catalog) / len(declared_subsystem_ids)`, clamped to
//! 1.0. Zero declared subsystems → vacuously 1.0.

use std::collections::HashSet;

use atlas_agents::events::Grade;
use atlas_agents::runtime::audit::lane_a::{lane_a_validate, AgentOutput, Stage};
use atlas_agents::runtime::Transcript;
use serde_json::json;

fn empty_candidates() -> HashSet<String> {
    HashSet::new()
}

fn synthetic_project_output(grade: &str, declared: &[&str], catalog: &[&str]) -> AgentOutput {
    AgentOutput::from_value(json!({
        "workspace_purpose": "Synthetic workspace.",
        "declared_subsystem_ids": declared,
        "subsystem_catalog": catalog.iter().map(|id| json!({
            "subsystem_id": id,
            "purpose": "x",
            "component_count": 1
        })).collect::<Vec<_>>(),
        "doc_scaffold": { "sections": [] },
        "confidence_grade": grade
    }))
}

#[tokio::test]
async fn project_claims_strong_vacuously_when_no_declared_subsystems() {
    let out = synthetic_project_output("strong", &[], &[]);
    let clamped = lane_a_validate(
        &out,
        Stage::Project,
        &empty_candidates(),
        &Transcript::new(),
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Strong);
}

#[tokio::test]
async fn project_claims_strong_with_full_coverage_stays_strong() {
    let out = synthetic_project_output("strong", &["agents", "cli"], &["agents", "cli"]);
    let clamped = lane_a_validate(
        &out,
        Stage::Project,
        &empty_candidates(),
        &Transcript::new(),
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Strong);
}

#[tokio::test]
async fn project_claims_strong_with_half_coverage_clamps_to_moderate() {
    let out = synthetic_project_output(
        "strong",
        &["agents", "cli", "engine", "reports"],
        &["agents", "cli"],
    );
    let clamped = lane_a_validate(
        &out,
        Stage::Project,
        &empty_candidates(),
        &Transcript::new(),
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}

#[tokio::test]
async fn project_claims_strong_with_zero_coverage_clamps_to_declines() {
    let out = synthetic_project_output("strong", &["agents", "cli"], &[]);
    let clamped = lane_a_validate(
        &out,
        Stage::Project,
        &empty_candidates(),
        &Transcript::new(),
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Declines);
}

#[tokio::test]
async fn project_llm_may_claim_lower_than_evidence_max() {
    let out = synthetic_project_output("moderate", &["agents", "cli"], &["agents", "cli"]);
    let clamped = lane_a_validate(
        &out,
        Stage::Project,
        &empty_candidates(),
        &Transcript::new(),
    )
    .await
    .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}
