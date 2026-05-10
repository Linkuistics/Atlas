//! `<output>/.atlas/config.yaml` schema (Atlas vNext Phase 1).
//!
//! Workspace-level configuration: discovered roots, model routing per
//! analyser stage, and override-search paths (design §6.7). Phase 1
//! ships `schema_version: 1`.
//!
//! This module is the on-disk schema only. The atlas-llm crate keeps
//! its own provider config (`crates/atlas-llm/src/config.rs`); a
//! future PR may unify the two, but Phase 1 keeps them separate and
//! `AtlasConfigFile.operations` mirrors the OperationConfig shape
//! (`model: String`, optional `params`) so the two don't drift.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Schema version for `config.yaml`.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Per-stage model routing entry. The shape mirrors the
/// atlas-llm crate's `OperationConfig` (`model` + optional `params`)
/// so the two on-disk forms stay aligned without coupling crates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRouting {
    /// Provider-qualified model id (e.g.
    /// `claude-code/claude-sonnet-4-6`,
    /// `openrouter/anthropic/claude-sonnet-4-6`).
    pub model: String,
    /// Open-ended provider-specific parameter bag (e.g. `max_tokens`,
    /// `temperature`). Skipped on serialise when empty.
    #[serde(default, skip_serializing_if = "is_empty_value")]
    pub params: serde_yaml::Value,
}

/// `serde_yaml::Value` doesn't expose a stable `is_empty`; treat
/// `Null` and an empty `Mapping` as empty. Used by skip-serialise.
fn is_empty_value(v: &serde_yaml::Value) -> bool {
    match v {
        serde_yaml::Value::Null => true,
        serde_yaml::Value::Mapping(m) => m.is_empty(),
        _ => false,
    }
}

/// Top-level shape of `<output>/.atlas/config.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasConfigFile {
    pub schema_version: u32,
    /// Discovered roots, including the primary root and every
    /// peer-root reached via path-dep walking (design §5.3, §6.7).
    /// Phase 1 writes them in iteration order; future PRs may sort.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<PathBuf>,
    /// Per-stage LLM routing. The map keys are stage names —
    /// `"classify"`, `"surface"`, `"edges"`, `"subcarve"` etc. —
    /// matching the atlas-llm `OperationsConfig` field names.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, ModelRouting>,
    /// Optional list of paths that the override loader searches in
    /// addition to the per-component `<component>/.atlas/overrides.yaml`
    /// convention. Useful for monorepos that prefer a single
    /// top-level overrides file over scattered per-component files
    /// (design §6.7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub override_search: Vec<PathBuf>,
}

impl Default for AtlasConfigFile {
    fn default() -> Self {
        AtlasConfigFile {
            schema_version: CONFIG_SCHEMA_VERSION,
            roots: Vec::new(),
            operations: BTreeMap::new(),
            override_search: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_atlas_config_file() -> AtlasConfigFile {
        let mut operations = BTreeMap::new();
        operations.insert(
            "classify".to_string(),
            ModelRouting {
                model: "claude-code/claude-sonnet-4-6".into(),
                params: serde_yaml::Value::Null,
            },
        );
        operations.insert(
            "surface".to_string(),
            ModelRouting {
                model: "claude-code/claude-sonnet-4-6".into(),
                params: serde_yaml::from_str("max_tokens: 2048\n").unwrap(),
            },
        );

        AtlasConfigFile {
            schema_version: CONFIG_SCHEMA_VERSION,
            roots: vec![
                PathBuf::from("/Users/antony/Development/Ravel-Lite"),
                PathBuf::from("/Users/antony/Development/atlas-contracts"),
            ],
            operations,
            override_search: vec![
                PathBuf::from(".atlas/overrides.yaml"),
                PathBuf::from("<component-path>/.atlas/overrides.yaml"),
            ],
        }
    }

    #[test]
    fn atlas_config_file_round_trips_through_yaml() {
        let original = sample_atlas_config_file();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: AtlasConfigFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn model_routing_with_params_round_trips() {
        let original = ModelRouting {
            model: "anthropic/claude-haiku-4-5".into(),
            params: serde_yaml::from_str("max_tokens: 1024\ntemperature: 0.0\n").unwrap(),
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: ModelRouting = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn model_routing_empty_params_skipped_in_yaml() {
        let original = ModelRouting {
            model: "claude-code/claude-sonnet-4-6".into(),
            params: serde_yaml::Value::Null,
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(!yaml.contains("params"), "got:\n{yaml}");
        let parsed: ModelRouting = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn atlas_config_file_default_has_current_schema_version() {
        let f = AtlasConfigFile::default();
        assert_eq!(f.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(f.roots.is_empty());
        assert!(f.operations.is_empty());
        assert!(f.override_search.is_empty());
    }

    #[test]
    fn atlas_config_file_default_serialises_without_empty_roots() {
        // The default round-trips without an empty `roots: []` line.
        // `roots` shares the skip-serialise hygiene already used by
        // `override_search` and `operations`.
        let original = AtlasConfigFile::default();
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(
            !yaml.contains("roots:"),
            "default config must not emit a `roots:` line, got:\n{yaml}"
        );
        let parsed: AtlasConfigFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }
}
