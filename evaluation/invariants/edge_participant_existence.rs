//! Structural invariant: every participant named in
//! `related-components.yaml` appears in `components.yaml` or
//! `external-components.yaml`.

use std::path::PathBuf;

use atlas_eval::edge_participant_existence;
use atlas_index::{
    CacheFingerprints, ComponentEntry, ComponentsFile, ExternalEntry, ExternalsFile, PathSegment,
    COMPONENTS_SCHEMA_VERSION, EXTERNALS_SCHEMA_VERSION,
};
use component_ontology::{
    Edge, EdgeKind, EvidenceGrade, LifecycleScope, RelatedComponentsFile,
};

fn component(id: &str) -> ComponentEntry {
    ComponentEntry {
        id: id.into(),
        parent: None,
        kind: "rust-library".into(),
        lifecycle_roles: vec![LifecycleScope::Build],
        language: Some("rust".into()),
        build_system: Some("cargo".into()),
        role: None,
        path_segments: vec![PathSegment {
            path: PathBuf::from(id),
            content_sha: "sha".into(),
        }],
        manifests: vec![],
        doc_anchors: vec![],
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["manifest".into()],
        rationale: "crate".into(),
        deleted: false,
    }
}

fn components(ids: &[&str]) -> ComponentsFile {
    ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: PathBuf::from("."),
        generated_at: String::new(),
        cache_fingerprints: CacheFingerprints::default(),
        components: ids.iter().map(|id| component(id)).collect(),
    }
}

fn externals(ids: &[&str]) -> ExternalsFile {
    ExternalsFile {
        schema_version: EXTERNALS_SCHEMA_VERSION,
        externals: ids
            .iter()
            .map(|id| ExternalEntry {
                id: (*id).into(),
                kind: "external".into(),
                language: Some("rust".into()),
                purl: None,
                homepage: None,
                url: None,
                discovered_from: vec!["Cargo.toml".into()],
                evidence_grade: EvidenceGrade::Strong,
            })
            .collect(),
    }
}

fn edge(a: &str, b: &str) -> Edge {
    Edge {
        kind: EdgeKind::DependsOn,
        lifecycle: LifecycleScope::Build,
        participants: vec![a.into(), b.into()],
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["manifest".into()],
        rationale: "dep".into(),
    }
}

#[test]
fn edge_participant_existence_accepts_valid_participants() {
    let c = components(&["a"]);
    let e = externals(&["crate:serde"]);
    let mut related = RelatedComponentsFile::default();
    related.add_edge(edge("a", "crate:serde")).unwrap();
    assert!(edge_participant_existence(&c, &e, &related).is_ok());
}

#[test]
fn edge_participant_existence_rejects_unknown_participant() {
    let c = components(&["a"]);
    let e = externals(&[]);
    let mut related = RelatedComponentsFile::default();
    related.add_edge(edge("a", "ghost")).unwrap();
    let err = edge_participant_existence(&c, &e, &related).unwrap_err();
    assert_eq!(err.invariant, "edge_participant_existence");
    assert!(err.message.contains("ghost"));
}
