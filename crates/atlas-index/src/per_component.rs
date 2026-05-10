//! `<component-path>/.atlas/component.yaml` schema (Atlas vNext
//! Phase 1).
//!
//! Per-component intrinsic record: the same `ComponentEntry` that
//! appears in the top-level `components.yaml`, plus pointers to the
//! co-located `surfaces.yaml` and optional `overrides.yaml`, plus
//! enough analyser / fingerprint metadata to re-derive the entry
//! without re-reading the top-level file (design §6.2).
//!
//! The file is the load-bearing record for the "data co-locates with
//! source" principle (§4.6): a vendored copy of a component brings
//! its `.atlas/component.yaml` along, and a host repo can read the
//! component's classification immediately.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::schema::ComponentEntry;

/// Schema version for `<component-path>/.atlas/component.yaml`.
pub const PER_COMPONENT_SCHEMA_VERSION: u32 = 1;

/// Top-level shape of `<component-path>/.atlas/component.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerComponentFile {
    pub schema_version: u32,
    /// The component's projected entry. Identical to its slot in the
    /// top-level `components.yaml`, modulo independent file write
    /// order — the top-level file is a synthesis of every
    /// per-component record.
    pub component: ComponentEntry,
    /// Path (relative to the per-component `.atlas/` directory) of
    /// the co-located surfaces file.
    pub surfaces_path: PathBuf,
    /// Optional override file co-located with the component
    /// (per-component overrides per the §11.2.3 resolution spec).
    /// `None` when the component carries no per-component overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides_path: Option<PathBuf>,
    /// Stable id of the L3 analyser whose verdict produced this
    /// entry — for example `cargo-toml-classifier`, `dockerfile-l3`,
    /// or (Phase 2) `python-surface-analyzer`,
    /// `typescript-package-classifier`, etc. The value is captured
    /// from the dispatching analyser's
    /// [`atlas_index::AnalyzerSpec::id`] (equivalently the analyser
    /// trait's `id()`), so a re-classify by a different analyser
    /// bumps the field and the per-component fingerprint.
    ///
    /// Two reserved sentinels mark provenance outside the analyser
    /// registry:
    ///
    /// - `"none"` — every applicable analyser declined; no analyser
    ///   produced the verdict. Paired with version `"0.0.0"`.
    /// - `"override"` — the classification came from a hand-authored
    ///   pin or `overrides.additions` entry rather than an analyser
    ///   run. Also paired with version `"0.0.0"`.
    ///
    /// PR-4 (Phase 2) replaces the historical `"l3-driver"`
    /// placeholder with these per-analyser values.
    pub analyser_id: String,
    /// Free-form analyser version string. Same provenance as
    /// [`atlas_index::AnalyzerSpec::version`] — the value reported
    /// by the analyser trait's `version()` for the dispatching
    /// analyser. `"0.0.0"` for the `"none"` / `"override"`
    /// sentinels described on `analyser_id`.
    pub analyser_version: String,
    /// SHA-256 hex of the input fingerprint that produced this
    /// entry. PR-2 (cache) lays out how this is computed; Phase 1
    /// records the value without acting on it beyond auditability.
    pub fingerprint: String,
}

impl Default for PerComponentFile {
    fn default() -> Self {
        PerComponentFile {
            schema_version: PER_COMPONENT_SCHEMA_VERSION,
            component: placeholder_entry(),
            surfaces_path: PathBuf::from("surfaces.yaml"),
            overrides_path: None,
            analyser_id: String::new(),
            analyser_version: String::new(),
            fingerprint: String::new(),
        }
    }
}

/// Construct a minimum-viable `ComponentEntry` for `Default`.
/// `ComponentEntry` does not implement `Default` (the id field is
/// non-empty and the evidence grade is non-default), so the
/// per-component default builds a placeholder explicitly.
fn placeholder_entry() -> ComponentEntry {
    use std::collections::BTreeSet;

    use component_ontology::{ComponentId, EvidenceGrade};

    ComponentEntry {
        id: ComponentId::parse("placeholder").expect("`placeholder` is a valid ComponentId"),
        parent: None,
        kind: "placeholder".into(),
        lifecycle_roles: Vec::new(),
        languages: BTreeSet::new(),
        build_system: None,
        role: None,
        path_segments: Vec::new(),
        manifests: Vec::new(),
        doc_anchors: Vec::new(),
        evidence_grade: EvidenceGrade::Weak,
        evidence_fields: Vec::new(),
        rationale: String::new(),
        deleted: false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::schema::PathSegment;
    use super::*;
    use component_ontology::{ComponentId, EvidenceGrade, LifecycleScope};

    fn sample_per_component_file() -> PerComponentFile {
        PerComponentFile {
            schema_version: PER_COMPONENT_SCHEMA_VERSION,
            component: ComponentEntry {
                id: ComponentId::parse("atlas-contracts/atlas-index").unwrap(),
                parent: Some(ComponentId::parse("atlas-contracts").unwrap()),
                kind: "rust-library".into(),
                lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
                languages: BTreeSet::from(["rust".to_string()]),
                build_system: Some("cargo".into()),
                role: Some("library".into()),
                path_segments: vec![PathSegment {
                    path: PathBuf::from("crates/atlas-index"),
                    content_sha: "abc123".into(),
                }],
                manifests: vec![PathBuf::from("crates/atlas-index/Cargo.toml")],
                doc_anchors: Vec::new(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec!["Cargo.toml:[lib]".into()],
                rationale: "has Cargo.toml [lib] section".into(),
                deleted: false,
            },
            surfaces_path: PathBuf::from("surfaces.yaml"),
            overrides_path: Some(PathBuf::from("overrides.yaml")),
            analyser_id: "cargo-toml-classifier".into(),
            analyser_version: "1.0.3".into(),
            fingerprint: "f".repeat(64),
        }
    }

    #[test]
    fn per_component_file_round_trips_through_yaml() {
        let original = sample_per_component_file();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: PerComponentFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn per_component_file_omits_none_overrides_path() {
        let mut original = sample_per_component_file();
        original.overrides_path = None;
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(
            !yaml.contains("overrides_path"),
            "missing overrides_path must skip the field, got:\n{yaml}"
        );
        let parsed: PerComponentFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn per_component_file_default_has_current_schema_version() {
        let f = PerComponentFile::default();
        assert_eq!(f.schema_version, PER_COMPONENT_SCHEMA_VERSION);
        assert_eq!(f.surfaces_path, PathBuf::from("surfaces.yaml"));
        assert!(f.overrides_path.is_none());
    }
}
