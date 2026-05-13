//! PR-3 round-trip test for the canonical-schema shim
//! ([`atlas_agents::runtime::projection_to_canonical`]).
//!
//! Builds a synthetic `L9Projection` with two subsystems + per-subsystem
//! children + a workspace-level project, runs the shim against a tempdir,
//! and asserts (a) all three canonical YAML files land on disk, (b) each
//! re-reads via its target canonical struct, and (c) the re-read struct
//! equals the in-memory result the shim returned.
//!
//! The round-trip guarantee is what makes the canonical artifacts safe
//! for downstream LLM consumers — they can re-read what the shim wrote
//! without information loss.

use std::collections::HashMap;

use atlas_agents::runtime::projection_to_canonical::{
    project_l9_to_canonical, CanonicalArtifactSet, ComponentsCanonical, RelatedComponentsCanonical,
    SubsystemsCanonical,
};
use atlas_agents::runtime::{AgentOutput, L9Projection};
use serde_json::json;
use tempfile::TempDir;

/// Build a synthetic `L9Projection` with two subsystems (`agents`,
/// `cli`), two components per subsystem, and a workspace-level project
/// output. Every required canonical field is populated.
fn synthetic_l9_two_subsystems() -> L9Projection {
    let mut components: HashMap<String, AgentOutput> = HashMap::new();
    components.insert(
        "atlas-agents".to_string(),
        AgentOutput::from_value(json!({
            "component_id": "atlas-agents",
            "kind": "rust-library",
            "language": "rust",
            "lifecycle": "runtime",
            "subsystem_hint": "agents",
            "evidence_pointers": [
                { "path": "crates/atlas-agents/Cargo.toml" }
            ],
            "confidence_grade": "strong"
        })),
    );
    components.insert(
        "atlas-llm".to_string(),
        AgentOutput::from_value(json!({
            "component_id": "atlas-llm",
            "kind": "rust-library",
            "language": "rust",
            "lifecycle": "runtime",
            "subsystem_hint": "agents",
            "evidence_pointers": [
                { "path": "crates/atlas-llm/Cargo.toml" }
            ],
            "confidence_grade": "strong"
        })),
    );
    components.insert(
        "atlas-cli".to_string(),
        AgentOutput::from_value(json!({
            "component_id": "atlas-cli",
            "kind": "rust-binary",
            "language": "rust",
            "lifecycle": "build",
            "subsystem_hint": "cli",
            "evidence_pointers": [
                { "path": "crates/atlas-cli/Cargo.toml" }
            ],
            "confidence_grade": "moderate"
        })),
    );
    components.insert(
        "atlas-engine".to_string(),
        AgentOutput::from_value(json!({
            "component_id": "atlas-engine",
            "kind": "rust-library",
            "language": "rust",
            "lifecycle": "build",
            "subsystem_hint": "cli",
            "evidence_pointers": [
                { "path": "crates/atlas-engine/Cargo.toml" }
            ],
            "confidence_grade": "moderate"
        })),
    );

    let mut subsystems: HashMap<String, AgentOutput> = HashMap::new();
    subsystems.insert(
        "agents".to_string(),
        AgentOutput::from_value(json!({
            "subsystem_id": "agents",
            "purpose": "Async LLM-spine runtime owning the per-stage tool loop and Lane A/B audits.",
            "component_ids": ["atlas-agents", "atlas-llm"],
            "key_contracts": [
                { "id": "tools/parse_cargo_toml", "kind": "tool-handle" }
            ],
            "internal_edges": [
                { "from": "atlas-agents", "to": "atlas-llm", "kind": "depends-on" }
            ],
            "refactoring_cues": [
                {
                    "kind": "abstraction-opportunity",
                    "component_ids": ["atlas-agents"],
                    "rationale": "Tool catalog could be split per stage."
                }
            ],
            "evidence_pointers": [
                { "path": "crates/atlas-agents/Cargo.toml" }
            ],
            "confidence_grade": "strong"
        })),
    );
    subsystems.insert(
        "cli".to_string(),
        AgentOutput::from_value(json!({
            "subsystem_id": "cli",
            "purpose": "CLI entry points and pipeline orchestration over the agent runtime.",
            "component_ids": ["atlas-cli", "atlas-engine"],
            "internal_edges": [
                { "from": "atlas-cli", "to": "atlas-engine", "kind": "depends-on" }
            ],
            "evidence_pointers": [
                { "path": "crates/atlas-cli/Cargo.toml" }
            ],
            "confidence_grade": "moderate"
        })),
    );

    let project = AgentOutput::from_value(json!({
        "workspace_purpose": "Atlas: LLM-spine monorepo analysis tool feeding downstream LLM consumers.",
        "subsystem_catalog": [
            { "subsystem_id": "agents", "purpose": "Async LLM-spine.", "component_count": 2 },
            { "subsystem_id": "cli", "purpose": "CLI orchestration.", "component_count": 2 }
        ],
        "cross_subsystem_edges": [
            { "from": "cli", "to": "agents", "kind": "depends-on" }
        ],
        "doc_scaffold": {
            "sections": [
                { "heading": "Architecture", "source_references": [], "child_sections": [] }
            ]
        },
        "confidence_grade": "moderate"
    }));

    L9Projection {
        components,
        subsystems,
        project: Some(project),
    }
}

#[test]
fn synthetic_l9_round_trips_through_canonical_yamls() {
    let tmp = TempDir::new().unwrap();
    let l9 = synthetic_l9_two_subsystems();
    let result: CanonicalArtifactSet =
        project_l9_to_canonical(&l9, tmp.path()).expect("shim must succeed on well-formed L9");

    // All three files exist on disk.
    let components_path = tmp.path().join("components.yaml");
    let subsystems_path = tmp.path().join("subsystems.yaml");
    let related_path = tmp.path().join("related-components.yaml");
    assert!(components_path.exists(), "components.yaml must exist");
    assert!(subsystems_path.exists(), "subsystems.yaml must exist");
    assert!(related_path.exists(), "related-components.yaml must exist");

    // Re-read each file and assert it round-trips into the canonical
    // struct + equals the in-memory result.
    let components_bytes = std::fs::read_to_string(&components_path).unwrap();
    let components_reread: ComponentsCanonical = serde_yaml::from_str(&components_bytes).unwrap();
    assert_eq!(result.components, components_reread);

    let subsystems_bytes = std::fs::read_to_string(&subsystems_path).unwrap();
    let subsystems_reread: SubsystemsCanonical = serde_yaml::from_str(&subsystems_bytes).unwrap();
    assert_eq!(result.subsystems, subsystems_reread);

    let related_bytes = std::fs::read_to_string(&related_path).unwrap();
    let related_reread: RelatedComponentsCanonical = serde_yaml::from_str(&related_bytes).unwrap();
    assert_eq!(result.related, related_reread);
}

#[test]
fn shim_emits_deterministic_component_order() {
    // HashMap iteration order is non-deterministic; the shim sorts
    // component ids before emitting so two runs against the same L9
    // produce byte-identical YAML.
    let l9 = synthetic_l9_two_subsystems();
    let tmp = TempDir::new().unwrap();
    let _ = project_l9_to_canonical(&l9, tmp.path()).unwrap();
    let bytes1 = std::fs::read_to_string(tmp.path().join("components.yaml")).unwrap();

    let tmp2 = TempDir::new().unwrap();
    let _ = project_l9_to_canonical(&l9, tmp2.path()).unwrap();
    let bytes2 = std::fs::read_to_string(tmp2.path().join("components.yaml")).unwrap();

    assert_eq!(
        bytes1, bytes2,
        "components.yaml must be deterministic across runs"
    );
}

#[test]
fn related_components_carries_cross_subsystem_edges() {
    let l9 = synthetic_l9_two_subsystems();
    let tmp = TempDir::new().unwrap();
    let result = project_l9_to_canonical(&l9, tmp.path()).unwrap();
    // 2 internal edges (one per subsystem) + 1 cross-subsystem edge = 3.
    assert_eq!(result.related.edges.len(), 3, "expected 3 edges total");
    assert!(
        result
            .related
            .edges
            .iter()
            .any(|e| e.source == "cross_subsystem"),
        "must include at least one cross_subsystem-tagged edge"
    );
    assert!(
        result
            .related
            .edges
            .iter()
            .any(|e| e.source.starts_with("internal:")),
        "must include at least one internal:<subsystem>-tagged edge"
    );
}

#[test]
fn shim_uses_atomic_write_pair_for_components_and_subsystems() {
    // Both sibling files must land together (the two-rename atomic-pair
    // semantic). PR-3 cannot test the crash-in-between window without
    // arming the engine's atomic_write panic-injection hook (which
    // requires a feature flag); instead we pin the happy-path: both
    // files exist after success.
    let tmp = TempDir::new().unwrap();
    let l9 = synthetic_l9_two_subsystems();
    let _ = project_l9_to_canonical(&l9, tmp.path()).unwrap();
    assert!(tmp.path().join("components.yaml").exists());
    assert!(tmp.path().join("subsystems.yaml").exists());
    assert!(tmp.path().join("related-components.yaml").exists());
}
