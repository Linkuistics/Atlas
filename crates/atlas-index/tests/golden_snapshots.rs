//! Golden-snapshot tests for the three Atlas-owned YAMLs.
//!
//! The first run of each test writes its snapshot to
//! `tests/snapshots/<name>` and fails with an instructive message; the
//! developer commits the file and the test passes on every subsequent
//! run. Drift — whether from a deliberate schema change or an
//! accidental renaming of a `serde(rename_all)` attribute — fails the
//! assertion with a diff, forcing an explicit update.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_index::{
    AlwaysTrue, CacheFingerprints, ComponentEntry, ComponentsFile, DocAnchor, ExternalEntry,
    ExternalsFile, OverridesFile, PathSegment, PinValue, COMPONENTS_SCHEMA_VERSION,
    EXTERNALS_SCHEMA_VERSION, OVERRIDES_SCHEMA_VERSION,
};
use component_ontology::{EvidenceGrade, LifecycleScope};

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn check_snapshot(name: &str, actual: &str) {
    let path = snapshot_dir().join(name);
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        panic!(
            "wrote new golden snapshot at {} — review, commit, re-run",
            path.display()
        );
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("failed to read snapshot {}: {e}", path.display())
    });
    assert_eq!(
        actual, expected,
        "snapshot drift for {name}; update tests/snapshots/{name} if the change is intentional"
    );
}

fn sample_component_entry() -> ComponentEntry {
    ComponentEntry {
        id: "atlas/component-ontology".into(),
        parent: Some("atlas".into()),
        kind: "rust-library".into(),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Design],
        language: Some("rust".into()),
        build_system: Some("cargo".into()),
        role: Some("library".into()),
        path_segments: vec![PathSegment {
            path: PathBuf::from("crates/component-ontology"),
            content_sha: "abc123".into(),
        }],
        manifests: vec![PathBuf::from("crates/component-ontology/Cargo.toml")],
        doc_anchors: vec![DocAnchor {
            path: PathBuf::from("crates/component-ontology/README.md"),
            heading: "component-ontology".into(),
        }],
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["Cargo.toml:[lib]".into()],
        rationale: "Cargo.toml declares a library crate".into(),
        deleted: false,
    }
}

#[test]
fn components_yaml_golden_snapshot() {
    let file = ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: PathBuf::from("/repo/atlas"),
        generated_at: "2026-04-24T00:00:00Z".into(),
        cache_fingerprints: CacheFingerprints {
            ontology_sha: "0000000000000000000000000000000000000000000000000000000000000001"
                .into(),
            prompt_shas: BTreeMap::from([
                (
                    "classify".to_string(),
                    "0000000000000000000000000000000000000000000000000000000000000002".to_string(),
                ),
                (
                    "stage1-surface".to_string(),
                    "0000000000000000000000000000000000000000000000000000000000000003".to_string(),
                ),
            ]),
            model_id: "claude-opus-4-7".into(),
            backend_version: "claude-cli 1.0.0".into(),
        },
        components: vec![sample_component_entry()],
    };
    let yaml = serde_yaml::to_string(&file).unwrap();
    check_snapshot("components.yaml", &yaml);
}

#[test]
fn overrides_yaml_golden_snapshot() {
    let mut pins: BTreeMap<String, BTreeMap<String, PinValue>> = BTreeMap::new();
    let mut pinned = BTreeMap::new();
    pinned.insert(
        "role".into(),
        PinValue::Value {
            value: "spec".into(),
            reason: Some("hand-curated docs repo".into()),
        },
    );
    pinned.insert(
        "suppress_children".into(),
        PinValue::SuppressChildren {
            suppress_children: vec!["atlas/old-crate".into(), "atlas/older-crate".into()],
        },
    );
    pinned.insert(
        "deleted".into(),
        PinValue::Suppress {
            suppress: AlwaysTrue,
        },
    );
    pins.insert("atlas".into(), pinned);

    let file = OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins,
        additions: vec![sample_component_entry()],
    };
    let yaml = serde_yaml::to_string(&file).unwrap();
    check_snapshot("components.overrides.yaml", &yaml);
}

#[test]
fn externals_yaml_golden_snapshot() {
    let file = ExternalsFile {
        schema_version: EXTERNALS_SCHEMA_VERSION,
        externals: vec![
            ExternalEntry {
                id: "crate:serde".into(),
                kind: "external".into(),
                language: Some("rust".into()),
                purl: Some("pkg:cargo/serde@1".into()),
                homepage: Some("https://serde.rs".into()),
                url: None,
                discovered_from: vec!["Cargo.toml".into()],
                evidence_grade: EvidenceGrade::Strong,
            },
            ExternalEntry {
                id: "npm:typescript".into(),
                kind: "external".into(),
                language: Some("typescript".into()),
                purl: None,
                homepage: None,
                url: Some("https://www.typescriptlang.org/".into()),
                discovered_from: vec!["package.json".into(), "apps/web/package.json".into()],
                evidence_grade: EvidenceGrade::Medium,
            },
        ],
    };
    let yaml = serde_yaml::to_string(&file).unwrap();
    check_snapshot("external-components.yaml", &yaml);
}
