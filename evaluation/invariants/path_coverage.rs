//! Structural invariant: every internal component has a non-empty
//! `path_segments` vector. Cross-checks what the unit tests assert,
//! but exercised through the public API so a breaking rename of
//! `atlas_eval::path_coverage` surfaces here.

use std::path::PathBuf;

use atlas_eval::path_coverage;
use atlas_index::{
    CacheFingerprints, ComponentEntry, ComponentsFile, PathSegment, COMPONENTS_SCHEMA_VERSION,
};
use component_ontology::{EvidenceGrade, LifecycleScope};

fn component(id: &str, path: &str) -> ComponentEntry {
    ComponentEntry {
        id: id.into(),
        parent: None,
        kind: "rust-library".into(),
        lifecycle_roles: vec![LifecycleScope::Build],
        language: Some("rust".into()),
        build_system: Some("cargo".into()),
        role: None,
        path_segments: vec![PathSegment {
            path: PathBuf::from(path),
            content_sha: "deadbeef".into(),
        }],
        manifests: vec![PathBuf::from(format!("{path}/Cargo.toml"))],
        doc_anchors: vec![],
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["Cargo.toml:[package]".into()],
        rationale: "self-contained crate".into(),
        deleted: false,
    }
}

fn file_of(components: Vec<ComponentEntry>) -> ComponentsFile {
    ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: PathBuf::from("."),
        generated_at: String::new(),
        cache_fingerprints: CacheFingerprints::default(),
        components,
    }
}

#[test]
fn path_coverage_passes_on_a_well_formed_file() {
    let f = file_of(vec![component("a", "a"), component("b", "b")]);
    assert!(path_coverage(&f).is_ok());
}

#[test]
fn path_coverage_fails_on_empty_path_segments() {
    let mut c = component("a", "a");
    c.path_segments.clear();
    let f = file_of(vec![c]);
    let err = path_coverage(&f).unwrap_err();
    assert_eq!(err.invariant, "path_coverage");
    assert!(err.message.contains("`a`"));
}
