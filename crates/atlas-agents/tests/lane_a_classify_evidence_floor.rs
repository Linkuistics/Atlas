//! PR-3 evidence-floor acceptance tests for the per-component
//! Classify stage. Mirrors PR-2's `lane_a_dispatch_evidence_floor.rs`
//! shape.
//!
//! Pins the classify-stage ladder (brainstorm §6.1):
//!
//! | reads observed                               | score | ceiling   |
//! |----------------------------------------------|-------|-----------|
//! | manifest + entrypoint + classify tool called | 1.0   | Strong    |
//! | manifest + classify tool called              | 0.6   | Moderate  |
//! | manifest only                                | 0.4   | Weak      |
//! | none                                         | 0.0   | Declines  |
//!
//! The classify rubric is asymmetric: a "Strong" grade requires
//! `manifest_read && entrypoint_read && classify_tool_called`. A lower
//! claim (Moderate / Weak) passes through unchanged when its
//! evidence-max ceiling is higher (the min-clamp).

use std::collections::HashSet;

use atlas_agents::events::Grade;
use atlas_agents::runtime::audit::lane_a::{lane_a_validate, AgentOutput, Stage};
use atlas_agents::runtime::Transcript;
use serde_json::json;

fn empty_candidates() -> HashSet<String> {
    HashSet::new()
}

fn synthetic_rust_classify(grade: &str) -> AgentOutput {
    AgentOutput::from_value(json!({
        "kind": "rust-library",
        "language": "rust",
        "lifecycle": "build",
        "evidence_pointers": [
            { "path": "crates/foo/Cargo.toml" },
            { "path": "crates/foo/src/lib.rs" }
        ],
        "confidence_grade": grade
    }))
}

fn manifest_and_entrypoint_transcript() -> Transcript {
    let mut t = Transcript::new();
    t.push_synthetic_tool_call(
        "parse_cargo_toml",
        json!({ "path": "crates/foo/Cargo.toml" }),
        json!({}),
    );
    t.push_synthetic_tool_call(
        "read_file",
        json!({ "path": "crates/foo/src/lib.rs" }),
        json!({}),
    );
    t
}

fn manifest_and_tool_only_transcript() -> Transcript {
    let mut t = Transcript::new();
    t.push_synthetic_tool_call(
        "parse_cargo_toml",
        json!({ "path": "crates/foo/Cargo.toml" }),
        json!({}),
    );
    t
}

fn manifest_only_transcript() -> Transcript {
    // Generic read of the manifest path; no `parse_cargo_toml` call.
    // This should score 0.4 (manifest_read=true, classify_tool_called=false).
    let mut t = Transcript::new();
    t.push_synthetic_tool_call(
        "read_file",
        json!({ "path": "crates/foo/Cargo.toml" }),
        json!({}),
    );
    t
}

#[tokio::test]
async fn classify_claims_strong_with_empty_transcript_clamps_to_declines() {
    let out = synthetic_rust_classify("strong");
    let transcript = Transcript::new();
    let clamped = lane_a_validate(&out, Stage::Classify, &empty_candidates(), &transcript)
        .await
        .expect("schema layer must pass for a well-formed claim envelope");
    assert_eq!(clamped, Grade::Declines);
}

#[tokio::test]
async fn classify_claims_strong_with_full_evidence_stays_strong() {
    let out = synthetic_rust_classify("strong");
    let transcript = manifest_and_entrypoint_transcript();
    let clamped = lane_a_validate(&out, Stage::Classify, &empty_candidates(), &transcript)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Strong);
}

#[tokio::test]
async fn classify_claims_strong_with_manifest_and_tool_only_clamps_to_moderate() {
    let out = synthetic_rust_classify("strong");
    let transcript = manifest_and_tool_only_transcript();
    let clamped = lane_a_validate(&out, Stage::Classify, &empty_candidates(), &transcript)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}

#[tokio::test]
async fn classify_claims_strong_with_manifest_only_clamps_to_weak() {
    let out = synthetic_rust_classify("strong");
    let transcript = manifest_only_transcript();
    let clamped = lane_a_validate(&out, Stage::Classify, &empty_candidates(), &transcript)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Weak);
}

#[tokio::test]
async fn classify_llm_may_claim_lower_than_evidence_max() {
    // Evidence supports Strong (1.0), claimed Moderate; clamp is
    // `min` so the LLM's lower self-grade wins.
    let out = synthetic_rust_classify("moderate");
    let transcript = manifest_and_entrypoint_transcript();
    let clamped = lane_a_validate(&out, Stage::Classify, &empty_candidates(), &transcript)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}
