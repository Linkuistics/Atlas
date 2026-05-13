//! PR-3 missing-field test for the canonical-schema shim. Each missing
//! required field surfaces as `ShimError::MissingProjectionField` with
//! the specific field name + L9 projection path. The error fires
//! BEFORE any disk write, so no partial-write residue lands on disk
//! (atomic_write_pair / atomic_write semantics intact).
//!
//! Covers the three canonical-artifact types: project (workspace_purpose),
//! components (kind), subsystems (purpose).

use std::collections::HashMap;

use atlas_agents::runtime::projection_to_canonical::{project_l9_to_canonical, ShimError};
use atlas_agents::runtime::{AgentOutput, L9Projection};
use serde_json::json;
use tempfile::TempDir;

fn valid_classify(id: &str, subsystem: &str) -> AgentOutput {
    AgentOutput::from_value(json!({
        "component_id": id,
        "kind": "rust-library",
        "language": "rust",
        "lifecycle": "runtime",
        "subsystem_hint": subsystem,
        "evidence_pointers": [],
        "confidence_grade": "moderate"
    }))
}

fn valid_reduce(id: &str, component_ids: &[&str]) -> AgentOutput {
    AgentOutput::from_value(json!({
        "subsystem_id": id,
        "purpose": "Synthetic purpose.",
        "component_ids": component_ids,
        "evidence_pointers": [],
        "confidence_grade": "moderate"
    }))
}

fn valid_project(subsystem_ids: &[&str]) -> AgentOutput {
    AgentOutput::from_value(json!({
        "workspace_purpose": "Synthetic workspace.",
        "subsystem_catalog": subsystem_ids.iter().map(|id| json!({
            "subsystem_id": id,
            "purpose": "x",
            "component_count": 1
        })).collect::<Vec<_>>(),
        "doc_scaffold": { "sections": [] },
        "confidence_grade": "moderate"
    }))
}

fn assert_no_partial_residue(tmp: &TempDir) {
    assert!(
        !tmp.path().join("components.yaml").exists(),
        "components.yaml must not exist after shim error (no partial-write residue)"
    );
    assert!(
        !tmp.path().join("subsystems.yaml").exists(),
        "subsystems.yaml must not exist after shim error"
    );
    assert!(
        !tmp.path().join("related-components.yaml").exists(),
        "related-components.yaml must not exist after shim error"
    );
}

#[test]
fn missing_workspace_purpose_surfaces_shim_error_no_disk_residue() {
    let tmp = TempDir::new().unwrap();

    // Project AgentOutput is present but its `workspace_purpose`
    // field is missing.
    let project_missing_purpose = AgentOutput::from_value(json!({
        // workspace_purpose intentionally absent
        "subsystem_catalog": [],
        "doc_scaffold": { "sections": [] },
        "confidence_grade": "declines"
    }));
    let mut components: HashMap<String, AgentOutput> = HashMap::new();
    components.insert("atlas-cli".to_string(), valid_classify("atlas-cli", "cli"));
    let mut subsystems: HashMap<String, AgentOutput> = HashMap::new();
    subsystems.insert("cli".to_string(), valid_reduce("cli", &["atlas-cli"]));
    let l9 = L9Projection {
        components,
        subsystems,
        project: Some(project_missing_purpose),
    };

    let result = project_l9_to_canonical(&l9, tmp.path());
    let err = result.unwrap_err();
    match err {
        ShimError::MissingProjectionField { field, ref path } => {
            assert_eq!(field, "workspace_purpose");
            assert_eq!(path, "project");
        }
        other => panic!("expected MissingProjectionField, got {other:?}"),
    }

    assert_no_partial_residue(&tmp);
}

#[test]
fn missing_component_kind_surfaces_shim_error_no_disk_residue() {
    let tmp = TempDir::new().unwrap();

    let mut components: HashMap<String, AgentOutput> = HashMap::new();
    components.insert(
        "broken-component".to_string(),
        AgentOutput::from_value(json!({
            "component_id": "broken-component",
            // kind intentionally absent
            "language": "rust",
            "lifecycle": "build"
        })),
    );
    let mut subsystems: HashMap<String, AgentOutput> = HashMap::new();
    subsystems.insert(
        "cli".to_string(),
        valid_reduce("cli", &["broken-component"]),
    );
    let l9 = L9Projection {
        components,
        subsystems,
        project: Some(valid_project(&["cli"])),
    };

    let err = project_l9_to_canonical(&l9, tmp.path()).unwrap_err();
    match err {
        ShimError::MissingProjectionField { field, ref path } => {
            assert_eq!(field, "kind");
            assert_eq!(path, "components.broken-component");
        }
        other => panic!("expected MissingProjectionField on kind, got {other:?}"),
    }

    assert_no_partial_residue(&tmp);
}

#[test]
fn missing_subsystem_purpose_surfaces_shim_error_no_disk_residue() {
    let tmp = TempDir::new().unwrap();

    let mut components: HashMap<String, AgentOutput> = HashMap::new();
    components.insert("atlas-cli".to_string(), valid_classify("atlas-cli", "cli"));
    let mut subsystems: HashMap<String, AgentOutput> = HashMap::new();
    subsystems.insert(
        "cli".to_string(),
        AgentOutput::from_value(json!({
            "subsystem_id": "cli",
            // purpose intentionally absent
            "component_ids": ["atlas-cli"]
        })),
    );
    let l9 = L9Projection {
        components,
        subsystems,
        project: Some(valid_project(&["cli"])),
    };

    let err = project_l9_to_canonical(&l9, tmp.path()).unwrap_err();
    match err {
        ShimError::MissingProjectionField { field, ref path } => {
            assert_eq!(field, "purpose");
            assert_eq!(path, "subsystems.cli");
        }
        other => panic!("expected MissingProjectionField on purpose, got {other:?}"),
    }

    assert_no_partial_residue(&tmp);
}

#[test]
fn missing_project_struct_entirely_surfaces_workspace_purpose_error() {
    // `l9.project = None` should also surface as
    // MissingProjectionField { field: "workspace_purpose" } — the
    // project agent simply didn't run / didn't produce output.
    let tmp = TempDir::new().unwrap();
    let l9 = L9Projection {
        components: HashMap::new(),
        subsystems: HashMap::new(),
        project: None,
    };
    let err = project_l9_to_canonical(&l9, tmp.path()).unwrap_err();
    assert!(matches!(
        err,
        ShimError::MissingProjectionField {
            field: "workspace_purpose",
            ..
        }
    ));
    assert_no_partial_residue(&tmp);
}
