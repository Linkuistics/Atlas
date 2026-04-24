//! Structural invariant: every manifest found by the conservative
//! walker is referenced by some component's `manifests` field.

use std::path::PathBuf;

use atlas_eval::{collect_manifests_conservative, manifest_coverage};
use atlas_index::{
    CacheFingerprints, ComponentEntry, ComponentsFile, PathSegment, COMPONENTS_SCHEMA_VERSION,
};
use component_ontology::{EvidenceGrade, LifecycleScope};
use tempfile::TempDir;

fn component(id: &str, path: &str, manifests: Vec<PathBuf>) -> ComponentEntry {
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
            content_sha: "sha".into(),
        }],
        manifests,
        doc_anchors: vec![],
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["Cargo.toml".into()],
        rationale: "crate".into(),
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
fn walker_finds_nested_manifests() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("a/src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("b/src")).unwrap();
    std::fs::write(tmp.path().join("a/Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(tmp.path().join("b/Cargo.toml"), "[package]\n").unwrap();

    let found = collect_manifests_conservative(tmp.path());
    assert_eq!(found.len(), 2, "found: {found:?}");
}

#[test]
fn manifest_coverage_accepts_covered_tree() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("a")).unwrap();
    std::fs::write(tmp.path().join("a/Cargo.toml"), "[package]\n").unwrap();

    let f = file_of(vec![component(
        "a",
        "a",
        vec![PathBuf::from("a/Cargo.toml")],
    )]);
    assert!(manifest_coverage(&f, tmp.path()).is_ok());
}

#[test]
fn manifest_coverage_rejects_uncovered_manifest() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("orphan")).unwrap();
    std::fs::write(tmp.path().join("orphan/Cargo.toml"), "[package]\n").unwrap();

    let f = file_of(vec![]);
    let err = manifest_coverage(&f, tmp.path()).unwrap_err();
    assert_eq!(err.invariant, "manifest_coverage");
    assert!(err.message.contains("orphan"));
}
