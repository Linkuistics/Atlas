//! Structural invariant: round-tripping a ComponentsFile through
//! `atlas_index::rename_match` against itself preserves identifiers,
//! and relocating (path change, same content) still reuses the id.

use std::path::PathBuf;

use atlas_eval::rename_round_trip_holds;
use atlas_index::{
    CacheFingerprints, ComponentEntry, ComponentsFile, PathSegment, COMPONENTS_SCHEMA_VERSION,
};
use component_ontology::{EvidenceGrade, LifecycleScope};

fn component_with_segments(id: &str, segments: Vec<(PathBuf, &str)>) -> ComponentEntry {
    ComponentEntry {
        id: id.into(),
        parent: None,
        kind: "rust-library".into(),
        lifecycle_roles: vec![LifecycleScope::Build],
        language: Some("rust".into()),
        build_system: Some("cargo".into()),
        role: None,
        path_segments: segments
            .into_iter()
            .map(|(path, sha)| PathSegment {
                path,
                content_sha: sha.into(),
            })
            .collect(),
        manifests: vec![],
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
fn rename_round_trip_passes_on_identical_snapshots() {
    let prior = file_of(vec![component_with_segments(
        "stable",
        vec![(PathBuf::from("pkg"), "sha-content")],
    )]);
    let new = prior.clone();
    assert!(rename_round_trip_holds(&prior, &new).is_ok());
}

#[test]
fn rename_round_trip_passes_when_path_changes_but_content_sha_stable() {
    let prior = file_of(vec![component_with_segments(
        "stable",
        vec![(PathBuf::from("old-pkg"), "sha-content")],
    )]);
    let new = file_of(vec![component_with_segments(
        "stable",
        vec![(PathBuf::from("new-pkg"), "sha-content")],
    )]);
    // Same id on both sides; rename_match pairs them via shared
    // content_sha and the id is preserved.
    assert!(rename_round_trip_holds(&prior, &new).is_ok());
}

#[test]
fn rename_round_trip_fails_when_matched_pair_has_differing_ids() {
    let prior = file_of(vec![component_with_segments(
        "old-id",
        vec![(PathBuf::from("pkg"), "sha-content")],
    )]);
    let new = file_of(vec![component_with_segments(
        "new-id",
        vec![(PathBuf::from("pkg"), "sha-content")],
    )]);
    // rename_match will pair these via content_sha; the invariant
    // should flag that the id drifted.
    let err = rename_round_trip_holds(&prior, &new).unwrap_err();
    assert_eq!(err.invariant, "rename_round_trip");
    assert!(err.message.contains("old-id") && err.message.contains("new-id"));
}
