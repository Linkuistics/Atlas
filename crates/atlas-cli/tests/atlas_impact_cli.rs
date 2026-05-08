//! Phase 3 PR-9 acceptance tests for `atlas impact <id>`.
//!
//! Each test seeds a tempdir with a synthetic
//! `<root>/.atlas/cache/components.yaml` and
//! `<root>/.atlas/cache/related-components.yaml`, then shells out to
//! the compiled `atlas` binary with `ATLAS_OUTPUT_DIR` pointed at the
//! tempdir's `.atlas/`. The four tests cover the canonical happy-path
//! formats (`--format human`, `--format json`), the not-found exit
//! code + `did you mean` stderr, and the clap-level rejection of
//! `--no-write` (which the report subcommand intentionally lacks).

use std::path::Path;

use assert_cmd::Command;
use atlas_index::{save_components_atomic, save_related_components_atomic};
use atlas_index::{ComponentsFile, RelatedComponentsFile};
use component_ontology::{ComponentId, Edge, EdgeKind, EvidenceGrade, LifecycleScope};
use predicates::prelude::PredicateBooleanExt;
use predicates::str;
use tempfile::TempDir;

fn atlas() -> Command {
    Command::cargo_bin("atlas").expect("atlas binary must be built")
}

/// Build a fixture with one defining component (`atlas-contracts/owner`)
/// and one consumer (`ravel-lite/api`), wired through a single
/// `defines-contract` + `consumes-contract` edge pair. Used by the
/// happy-path tests.
fn write_fixture(root: &Path) {
    let cache_dir = root.join(".atlas/cache");
    std::fs::create_dir_all(&cache_dir).unwrap();

    let owner = atlas_index::ComponentEntry {
        id: ComponentId::parse("atlas-contracts/owner").unwrap(),
        parent: None,
        kind: "rust-library".into(),
        lifecycle_roles: vec![LifecycleScope::Runtime],
        languages: ["rust".to_string()].into_iter().collect(),
        build_system: Some("cargo".into()),
        role: None,
        path_segments: vec![atlas_index::PathSegment {
            path: std::path::PathBuf::from("atlas-contracts/owner"),
            content_sha: "sha256:0".into(),
        }],
        manifests: Vec::new(),
        doc_anchors: Vec::new(),
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["fixture".into()],
        rationale: "fixture".into(),
        deleted: false,
    };
    let api = atlas_index::ComponentEntry {
        id: ComponentId::parse("ravel-lite/api").unwrap(),
        parent: None,
        kind: "typescript-package".into(),
        lifecycle_roles: vec![LifecycleScope::Runtime],
        languages: ["typescript".to_string()].into_iter().collect(),
        build_system: Some("npm".into()),
        role: None,
        path_segments: vec![atlas_index::PathSegment {
            path: std::path::PathBuf::from("ravel-lite/api"),
            content_sha: "sha256:0".into(),
        }],
        manifests: Vec::new(),
        doc_anchors: Vec::new(),
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["fixture".into()],
        rationale: "fixture".into(),
        deleted: false,
    };
    let components = ComponentsFile {
        schema_version: atlas_index::COMPONENTS_SCHEMA_VERSION,
        roots: vec![root.to_path_buf()],
        generated_at: String::new(),
        cache_fingerprints: atlas_index::CacheFingerprints::default(),
        components: vec![owner, api],
    };
    save_components_atomic(&cache_dir.join("components.yaml"), &components).unwrap();

    let mut related = RelatedComponentsFile {
        schema_version: 2,
        edges: Vec::new(),
    };
    related
        .add_edge(Edge {
            kind: EdgeKind::DefinesContract,
            lifecycle: LifecycleScope::Design,
            participants: vec![
                "atlas-contracts/owner".into(),
                "atlas-contracts/index-schema/v1".into(),
            ],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["surfaces.yaml:contracts_defined".into()],
            rationale: "fixture".into(),
        })
        .unwrap();
    related
        .add_edge(Edge {
            kind: EdgeKind::ConsumesContract,
            lifecycle: LifecycleScope::Runtime,
            participants: vec![
                "ravel-lite/api".into(),
                "atlas-contracts/index-schema/v1".into(),
            ],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["surfaces.yaml:contracts_consumed".into()],
            rationale: "fixture".into(),
        })
        .unwrap();
    save_related_components_atomic(&cache_dir.join("related-components.yaml"), &related).unwrap();
}

#[test]
fn atlas_impact_known_target_human_format() {
    let tmp = TempDir::new().unwrap();
    write_fixture(tmp.path());
    atlas()
        .env("ATLAS_OUTPUT_DIR", tmp.path().join(".atlas"))
        .args([
            "impact",
            "atlas-contracts/index-schema/v1",
            "--format",
            "human",
        ])
        .assert()
        .success()
        // The human format prints an indented tree; assert against the
        // header + the consumer entry.
        .stdout(str::contains("impact of contract"))
        .stdout(str::contains("ravel-lite/api"))
        .stdout(str::contains("transitive consumers"));
}

#[test]
fn atlas_impact_json_format() {
    let tmp = TempDir::new().unwrap();
    write_fixture(tmp.path());
    let output = atlas()
        .env("ATLAS_OUTPUT_DIR", tmp.path().join(".atlas"))
        .args([
            "impact",
            "atlas-contracts/index-schema/v1",
            "--format",
            "json",
        ])
        .output()
        .expect("run atlas impact");
    assert!(output.status.success(), "atlas impact failed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected valid JSON, got error: {e}\n--- stdout:\n{stdout}"));
    // Spot-check the top-level shape against the design exemplar.
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["target"]["kind"], "contract");
    assert_eq!(parsed["target"]["id"], "atlas-contracts/index-schema/v1");
    let direct = parsed["direct_consumers"]
        .as_array()
        .expect("direct_consumers is array");
    assert_eq!(direct.len(), 1);
    assert_eq!(direct[0], "ravel-lite/api");
    assert!(parsed["partitions"]["by_language"].is_object());
    assert!(parsed["partitions"]["by_deploy_graph"].is_object());
    assert!(parsed["partitions"]["by_lifecycle"].is_object());
    assert_eq!(parsed["summary"]["direct_count"], 1);
    assert_eq!(parsed["summary"]["transitive_count"], 1);
}

#[test]
fn atlas_impact_target_not_found_exits_2() {
    let tmp = TempDir::new().unwrap();
    write_fixture(tmp.path());
    atlas()
        .env("ATLAS_OUTPUT_DIR", tmp.path().join(".atlas"))
        // "ravel-lite/ap" is exactly one deletion from "ravel-lite/api"
        // (a known component id) so the suggestion fires.
        .args(["impact", "ravel-lite/ap"])
        .assert()
        .code(2)
        .stderr(str::contains("target not found"))
        .stderr(str::contains("did you mean"))
        .stderr(str::contains("ravel-lite/api"));
}

#[test]
fn atlas_impact_no_write_flag_rejected() {
    // `--no-write` is not declared on `ImpactArgs`; clap rejects it
    // before the handler runs. Exit code is non-zero (clap uses 2 for
    // arg-parse failures, matching standard CLI conventions).
    atlas()
        .args(["impact", "--no-write", "atlas-contracts/index-schema/v1"])
        .assert()
        .failure()
        .stderr(str::contains("--no-write").or(str::contains("unexpected argument")));
}
