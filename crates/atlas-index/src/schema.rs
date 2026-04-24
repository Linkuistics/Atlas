//! Types for the four YAMLs that sit at the Atlas/Ravel-Lite boundary:
//! `components.yaml`, `components.overrides.yaml`,
//! `external-components.yaml`, and `related-components.yaml`.
//!
//! The first three are owned by this crate. The fourth is owned by
//! `component-ontology` and re-exported here so consumers only need one
//! crate. Each generated file carries its own `schema_version` —
//! independent versions let us evolve one file without forcing a reader
//! to relearn all four at once.
//!
//! `kind`, `role`, `language`, and `build_system` are kept as `String`
//! at this layer. The typed `ComponentKind` enum lands in `atlas-engine`
//! (see backlog task 5); anchoring the vocabulary to a not-yet-written
//! enum here would churn every downstream consumer every time the
//! vocabulary grows.

use std::collections::BTreeMap;
use std::path::PathBuf;

use component_ontology::{EvidenceGrade, LifecycleScope};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const COMPONENTS_SCHEMA_VERSION: u32 = 1;
pub const OVERRIDES_SCHEMA_VERSION: u32 = 1;
pub const EXTERNALS_SCHEMA_VERSION: u32 = 1;

/// Fingerprint set written into `components.yaml` so a second tool run
/// can detect whether any cache-invalidating input changed. `prompt_shas`
/// is keyed by prompt id (e.g., `"classify"`, `"stage1-surface"`); all
/// SHAs are stored as lowercase hex strings to keep the YAML diffable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheFingerprints {
    pub ontology_sha: String,
    #[serde(default)]
    pub prompt_shas: BTreeMap<String, String>,
    pub model_id: String,
    pub backend_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathSegment {
    /// Relative to `ComponentsFile::root`.
    pub path: PathBuf,
    /// Hex-encoded SHA256 of the directory tree at `path`. Rename-match
    /// compares prior vs. new segments by this value (see
    /// `rename_match.rs`).
    pub content_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocAnchor {
    /// Relative to `ComponentsFile::root`.
    pub path: PathBuf,
    /// ATX heading text (no leading `#` marks).
    pub heading: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub lifecycle_roles: Vec<LifecycleScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub path_segments: Vec<PathSegment>,
    #[serde(default)]
    pub manifests: Vec<PathBuf>,
    #[serde(default)]
    pub doc_anchors: Vec<DocAnchor>,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    pub rationale: String,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentsFile {
    pub schema_version: u32,
    pub root: PathBuf,
    pub generated_at: String,
    pub cache_fingerprints: CacheFingerprints,
    #[serde(default)]
    pub components: Vec<ComponentEntry>,
}

impl Default for ComponentsFile {
    fn default() -> Self {
        ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            root: PathBuf::new(),
            generated_at: String::new(),
            cache_fingerprints: CacheFingerprints::default(),
            components: Vec::new(),
        }
    }
}

/// Pin value for `components.overrides.yaml`.
///
/// The variants are discriminated by field name rather than a type tag
/// so a hand-written override file reads naturally. Serde tries the
/// variants in declaration order; `Suppress` and `SuppressChildren`
/// precede `Value` because `Value`'s single required field (`value`)
/// would otherwise greedily accept inputs like `{suppress: true}` (no
/// `value` field; fails) after already reading through the more
/// distinctive fields. In practice each variant is uniquely identified
/// by its key, so the ordering is defensive rather than load-bearing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PinValue {
    Suppress {
        suppress: AlwaysTrue,
    },
    SuppressChildren {
        suppress_children: Vec<String>,
    },
    Value {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// Marker that serialises as the literal `true` and rejects anything
/// else on deserialisation. Required because the `Suppress` variant
/// above must accept only `suppress: true` — silently accepting
/// `suppress: false` would let users disable a pin by typo.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AlwaysTrue;

impl Serialize for AlwaysTrue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        true.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AlwaysTrue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = bool::deserialize(deserializer)?;
        if !v {
            return Err(serde::de::Error::custom(
                "`suppress: false` is not meaningful; remove the pin instead",
            ));
        }
        Ok(AlwaysTrue)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverridesFile {
    pub schema_version: u32,
    /// Outer key: component id. Inner key: field name being pinned
    /// (e.g., `"role"`, `"kind"`, `"deleted"`). `BTreeMap` keeps the
    /// serialised output deterministic.
    #[serde(default)]
    pub pins: BTreeMap<String, BTreeMap<String, PinValue>>,
    /// Components authored by hand when no evidence exists for them
    /// (e.g., a spec directory with no manifest). They bypass L2/L3 and
    /// land in `components.yaml` directly.
    #[serde(default)]
    pub additions: Vec<ComponentEntry>,
}

impl Default for OverridesFile {
    fn default() -> Self {
        OverridesFile {
            schema_version: OVERRIDES_SCHEMA_VERSION,
            pins: BTreeMap::new(),
            additions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEntry {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Package URL (pURL) spec identifier, when a manifest supplies one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Manifest paths (relative to the repo root) that surfaced this
    /// external. Multiple entries are expected — the same crate can
    /// show up in several `Cargo.toml`s.
    #[serde(default)]
    pub discovered_from: Vec<String>,
    pub evidence_grade: EvidenceGrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub externals: Vec<ExternalEntry>,
}

impl Default for ExternalsFile {
    fn default() -> Self {
        ExternalsFile {
            schema_version: EXTERNALS_SCHEMA_VERSION,
            externals: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_value_value_variant_round_trips() {
        let pin = PinValue::Value {
            value: "library".into(),
            reason: Some("override the classifier".into()),
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_value_variant_omits_none_reason_in_yaml() {
        let pin = PinValue::Value {
            value: "library".into(),
            reason: None,
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        assert!(
            !yaml.contains("reason"),
            "reason: null should be skipped, got:\n{yaml}"
        );
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_suppress_variant_round_trips() {
        let pin = PinValue::Suppress {
            suppress: AlwaysTrue,
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        assert!(yaml.contains("suppress: true"), "got:\n{yaml}");
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_suppress_children_variant_round_trips() {
        let pin = PinValue::SuppressChildren {
            suppress_children: vec!["a".into(), "b".into()],
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_suppress_rejects_false() {
        let err = serde_yaml::from_str::<PinValue>("suppress: false").unwrap_err();
        let msg = err.to_string();
        // untagged enum wraps the inner error; the specific "remove the
        // pin instead" hint comes from our AlwaysTrue impl.
        assert!(
            msg.contains("data did not match")
                || msg.contains("not meaningful")
                || msg.contains("untagged"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn components_file_default_has_current_schema_version() {
        let f = ComponentsFile::default();
        assert_eq!(f.schema_version, COMPONENTS_SCHEMA_VERSION);
        assert!(f.components.is_empty());
    }

    #[test]
    fn overrides_file_default_has_current_schema_version() {
        let f = OverridesFile::default();
        assert_eq!(f.schema_version, OVERRIDES_SCHEMA_VERSION);
    }

    #[test]
    fn externals_file_default_has_current_schema_version() {
        let f = ExternalsFile::default();
        assert_eq!(f.schema_version, EXTERNALS_SCHEMA_VERSION);
    }

    #[test]
    fn component_entry_round_trips_through_yaml() {
        let entry = ComponentEntry {
            id: "workspace/crate-a".into(),
            parent: Some("workspace".into()),
            kind: "rust-library".into(),
            lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
            language: Some("rust".into()),
            build_system: Some("cargo".into()),
            role: Some("library".into()),
            path_segments: vec![PathSegment {
                path: PathBuf::from("crates/crate-a"),
                content_sha: "abc123".into(),
            }],
            manifests: vec![PathBuf::from("crates/crate-a/Cargo.toml")],
            doc_anchors: vec![DocAnchor {
                path: PathBuf::from("crates/crate-a/README.md"),
                heading: "Crate A".into(),
            }],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["Cargo.toml:[lib]".into()],
            rationale: "has Cargo.toml [lib] section".into(),
            deleted: false,
        };
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let parsed: ComponentEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn external_entry_round_trips_through_yaml() {
        let entry = ExternalEntry {
            id: "crate:serde".into(),
            kind: "external".into(),
            language: Some("rust".into()),
            purl: Some("pkg:cargo/serde@1".into()),
            homepage: None,
            url: None,
            discovered_from: vec!["Cargo.toml".into()],
            evidence_grade: EvidenceGrade::Strong,
        };
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let parsed: ExternalEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, entry);
    }
}
