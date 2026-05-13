//! PR-3 evidence-floor acceptance tests for the Surface stage.
//!
//! Surface evidence is a ratio: inspected paths (transcript reads
//! matching declared `surfaces[i].source_path`) plus `find_pub_items`
//! tool calls, all divided by declared surface count, clamped to 1.0.
//!
//! Special case: zero declared surfaces → 1.0 (vacuously satisfied).
//!
//! Lane A's schema layer for `Stage::Surface` requires at least one
//! surface entry; we satisfy that via a non-empty `surfaces` array.

use std::collections::HashSet;

use atlas_agents::events::Grade;
use atlas_agents::runtime::audit::lane_a::{lane_a_validate, AgentOutput, Stage};
use atlas_agents::runtime::Transcript;
use serde_json::json;

fn empty_candidates() -> HashSet<String> {
    HashSet::new()
}

fn synthetic_surface_output(grade: &str, surfaces: serde_json::Value) -> AgentOutput {
    AgentOutput::from_value(json!({
        "surfaces": surfaces,
        "confidence_grade": grade,
    }))
}

#[tokio::test]
async fn surface_claims_strong_with_empty_transcript_clamps_to_declines() {
    let out = synthetic_surface_output(
        "strong",
        json!([
            { "name": "GetWidget", "source_path": "src/widgets.rs" },
            { "name": "PutWidget", "source_path": "src/widgets.rs" }
        ]),
    );
    let transcript = Transcript::new();
    let clamped = lane_a_validate(&out, Stage::Surface, &empty_candidates(), &transcript)
        .await
        .expect("schema layer must pass when surfaces is non-empty");
    assert_eq!(clamped, Grade::Declines);
}

#[tokio::test]
async fn surface_claims_strong_with_all_paths_read_stays_strong() {
    let out = synthetic_surface_output(
        "strong",
        json!([
            { "name": "GetWidget", "source_path": "src/widgets.rs" },
            { "name": "Submit", "source_path": "src/cmd.rs" }
        ]),
    );
    let mut t = Transcript::new();
    t.push_synthetic_tool_call("read_file", json!({ "path": "src/widgets.rs" }), json!({}));
    t.push_synthetic_tool_call("read_file", json!({ "path": "src/cmd.rs" }), json!({}));
    let clamped = lane_a_validate(&out, Stage::Surface, &empty_candidates(), &t)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Strong);
}

#[tokio::test]
async fn surface_claims_strong_with_half_paths_read_clamps_to_moderate() {
    // Two declared surfaces; one source_path read → ratio = 0.5 →
    // ceiling = Moderate.
    let out = synthetic_surface_output(
        "strong",
        json!([
            { "name": "GetWidget", "source_path": "src/widgets.rs" },
            { "name": "Submit", "source_path": "src/cmd.rs" }
        ]),
    );
    let mut t = Transcript::new();
    t.push_synthetic_tool_call("read_file", json!({ "path": "src/widgets.rs" }), json!({}));
    let clamped = lane_a_validate(&out, Stage::Surface, &empty_candidates(), &t)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}

#[tokio::test]
async fn surface_llm_may_claim_lower_than_evidence_max() {
    // Full evidence (ratio = 1.0, ceiling = Strong); claimed Moderate
    // → clamp `min` preserves Moderate.
    let out = synthetic_surface_output(
        "moderate",
        json!([
            { "name": "GetWidget", "source_path": "src/widgets.rs" }
        ]),
    );
    let mut t = Transcript::new();
    t.push_synthetic_tool_call("read_file", json!({ "path": "src/widgets.rs" }), json!({}));
    let clamped = lane_a_validate(&out, Stage::Surface, &empty_candidates(), &t)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Moderate);
}

#[tokio::test]
async fn surface_with_find_pub_items_calls_counts_toward_coverage() {
    // The `find_pub_items` tool counts the same as a direct path read.
    // Three declared surfaces, three `find_pub_items` calls → ratio = 1.0.
    let out = synthetic_surface_output(
        "strong",
        json!([
            { "name": "A" },
            { "name": "B" },
            { "name": "C" }
        ]),
    );
    let mut t = Transcript::new();
    for _ in 0..3 {
        t.push_synthetic_tool_call("find_pub_items", json!({}), json!({}));
    }
    let clamped = lane_a_validate(&out, Stage::Surface, &empty_candidates(), &t)
        .await
        .unwrap();
    assert_eq!(clamped, Grade::Strong);
}
