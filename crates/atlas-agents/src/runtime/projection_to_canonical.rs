//! Map `L9Projection` (the LLM-spine runtime's intermediate workspace
//! summary) into the canonical YAML artifact set that downstream Atlas
//! tooling (framing #2 — other LLM tools) reads.
//!
//! The shim emits three sibling YAMLs under `output_dir`:
//!
//! - `components.yaml` — flat per-component rows (id, kind, language,
//!   lifecycle, subsystem, evidence pointers, confidence grade).
//! - `subsystems.yaml` — per-subsystem rows (id, purpose, component
//!   ids, key contracts, refactoring cues, evidence pointers,
//!   confidence grade).
//! - `related-components.yaml` — edges (from, to, kind) collected from
//!   per-subsystem `internal_edges` and the workspace-level
//!   `cross_subsystem_edges`.
//!
//! Hard-fail on a missing required field: `ShimError::MissingProjectionField`
//! errors are *intentionally noisy* — they're the prompt-correctness
//! oracle (brainstorm framing #2). If a project / classify / reduce
//! prompt didn't produce enough info to populate canonical fields,
//! the prompt is wrong, not the shim. The shim checks ALL required
//! fields BEFORE any disk write, so a failure leaves no
//! partial-write residue on disk.
//!
//! # New canonical struct ownership
//!
//! The grep at PR-3 plan-time found no pre-existing
//! `ComponentsYaml` / `SubsystemsYaml` / `RelatedComponentsYaml`
//! structs (the deterministic engine's
//! `atlas_index::{ComponentsFile, SubsystemsFile}` and
//! `component_ontology::RelatedComponentsFile` carry engine-only
//! fields like `path_segments` / `cache_fingerprints` / `doc_anchors`
//! that the LLM-spine has no business populating). This module is
//! therefore the canonical owner of the LLM-spine's `*Canonical`
//! shapes — a deliberately narrower set tailored to what the four
//! producer prompts actually emit, per memory
//! `feedback_no_deterministic_engine_comparison` (the shim isn't a
//! deterministic-engine comparison harness; it's a producer-prompt
//! correctness signal).

use std::path::{Path, PathBuf};

use atlas_engine::{atomic_write, atomic_write_pair};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::runtime::L9Projection;

/// Canonical YAML schema version emitted by this shim. Independent of
/// the engine's per-file schema versions (`COMPONENTS_SCHEMA_VERSION`,
/// `SUBSYSTEMS_SCHEMA_VERSION`) because the LLM-spine's canonical
/// shapes carry a different (narrower) field set; consumers
/// disambiguate via the file content.
pub const LLM_CANONICAL_SCHEMA_VERSION: u32 = 1;

/// Errors surfaced by [`project_l9_to_canonical`]. Each variant carries
/// enough information for the caller (PR-5 calibration; downstream
/// LLM consumers) to identify the producer-prompt that needs a
/// revision.
#[derive(Debug, Error)]
pub enum ShimError {
    /// A required canonical field is missing from `L9Projection`. The
    /// `field` names the canonical field that wasn't populated; the
    /// `path` locates the L9 sub-projection that should have carried
    /// it (e.g. `"components.atlas-cli"`, `"subsystems.agents"`,
    /// `"project"`).
    #[error("L9Projection missing required canonical field `{field}` at projection path `{path}`")]
    MissingProjectionField { field: &'static str, path: String },
    /// `serde_yaml::to_string` failed on one of the canonical
    /// structs. Should never fire in practice — every field is a
    /// pure POD shape — but surfaced rather than `.unwrap()`'d so a
    /// future struct extension that breaks YAML serializability is
    /// caught loudly.
    #[error("yaml serialization failed for {target}: {source}")]
    YamlSerialize {
        target: &'static str,
        #[source]
        source: serde_yaml::Error,
    },
    /// Filesystem write failure during the atomic-write pair / single
    /// write. The caller knows whether a half-pair window survives by
    /// inspecting `path` against the documented two-rename semantic.
    #[error("filesystem write failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The bundle returned by [`project_l9_to_canonical`] on success.
///
/// Carries the in-memory canonical structs so the caller can do
/// further processing without re-reading the freshly-written files.
/// `Eq` is derived so the round-trip test can assert that
/// "re-read from disk = in-memory result" by struct equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalArtifactSet {
    pub components: ComponentsCanonical,
    pub subsystems: SubsystemsCanonical,
    pub related: RelatedComponentsCanonical,
}

/// On-disk shape for `components.yaml` (LLM-spine variant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentsCanonical {
    pub schema_version: u32,
    pub components: Vec<CanonicalComponent>,
}

/// One row of `components.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalComponent {
    pub id: String,
    pub kind: String,
    pub language: String,
    pub lifecycle: String,
    pub subsystem: String,
    #[serde(default)]
    pub evidence_pointers: Vec<CanonicalEvidencePointer>,
    pub confidence_grade: String,
}

/// On-disk shape for `subsystems.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemsCanonical {
    pub schema_version: u32,
    pub subsystems: Vec<CanonicalSubsystem>,
}

/// One row of `subsystems.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalSubsystem {
    pub id: String,
    pub purpose: String,
    #[serde(default)]
    pub component_ids: Vec<String>,
    #[serde(default)]
    pub key_contracts: Vec<CanonicalContract>,
    #[serde(default)]
    pub refactoring_cues: Vec<CanonicalRefactoringCue>,
    #[serde(default)]
    pub evidence_pointers: Vec<CanonicalEvidencePointer>,
    pub confidence_grade: String,
}

/// On-disk shape for `related-components.yaml` (LLM-spine variant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedComponentsCanonical {
    pub schema_version: u32,
    pub edges: Vec<CanonicalEdge>,
}

/// One edge row. Source carries which subsystem reduce surfaced the
/// edge (`internal:<subsystem>`) or the literal `cross_subsystem`
/// marker for project-stage edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub source: String,
}

/// Flattened evidence pointer (path only; the LLM-spine drops
/// `line_range` to keep the canonical YAML diff-friendly across
/// re-runs — downstream consumers re-derive line info if they need
/// it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEvidencePointer {
    pub path: String,
}

/// Flattened contract reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalContract {
    pub id: String,
    pub kind: String,
}

/// Flattened refactoring cue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalRefactoringCue {
    pub kind: String,
    #[serde(default)]
    pub component_ids: Vec<String>,
    pub rationale: String,
}

/// Project `l9` into the three canonical YAML artifacts and write
/// them atomically under `output_dir`.
///
/// Sequence:
///
/// 1. Build all three in-memory canonical structs from `l9`. Every
///    `ShimError::MissingProjectionField` fires here, BEFORE any
///    disk write touches `output_dir`.
/// 2. `serde_yaml::to_string` each canonical struct.
/// 3. `atomic_write_pair(components.yaml, subsystems.yaml)` — these
///    must move together (recast §6.3's two-rename semantic) so the
///    canonical set never lands half-rolled.
/// 4. `atomic_write(related-components.yaml)` — the third file is
///    written separately. The residual half-triplet window (post the
///    pair-rename, pre the third rename) is detectable on next read
///    via per-file schema-version checks.
pub fn project_l9_to_canonical(
    l9: &L9Projection,
    output_dir: &Path,
) -> Result<CanonicalArtifactSet, ShimError> {
    // ---- Phase 1: build (every MissingProjectionField fires here) ----

    let components = build_components_canonical(l9)?;
    let subsystems = build_subsystems_canonical(l9)?;
    let related = build_related_components_canonical(l9)?;

    // ---- Phase 2: serialize ------------------------------------------

    let components_bytes =
        serde_yaml::to_string(&components).map_err(|e| ShimError::YamlSerialize {
            target: "components.yaml",
            source: e,
        })?;
    let subsystems_bytes =
        serde_yaml::to_string(&subsystems).map_err(|e| ShimError::YamlSerialize {
            target: "subsystems.yaml",
            source: e,
        })?;
    let related_bytes = serde_yaml::to_string(&related).map_err(|e| ShimError::YamlSerialize {
        target: "related-components.yaml",
        source: e,
    })?;

    // ---- Phase 3: write ----------------------------------------------

    let components_path = output_dir.join("components.yaml");
    let subsystems_path = output_dir.join("subsystems.yaml");
    let related_path = output_dir.join("related-components.yaml");

    atomic_write_pair(
        &components_path,
        components_bytes.as_bytes(),
        &subsystems_path,
        subsystems_bytes.as_bytes(),
    )
    .map_err(|e| ShimError::Io {
        path: components_path.clone(),
        source: e,
    })?;
    atomic_write(&related_path, related_bytes.as_bytes()).map_err(|e| ShimError::Io {
        path: related_path.clone(),
        source: e,
    })?;

    Ok(CanonicalArtifactSet {
        components,
        subsystems,
        related,
    })
}

/// Build the `components.yaml` canonical struct from `l9.components`.
///
/// Required fields per component (hard-fail on missing): `kind`,
/// `language`, `lifecycle`. The runtime-assigned `subsystem` is
/// derived from `subsystem_hint` when present; otherwise the empty
/// string (LLM declined to commit). `evidence_pointers` and
/// `confidence_grade` carry empty / default values when missing
/// rather than hard-failing — they're informational fields, not
/// structural.
fn build_components_canonical(l9: &L9Projection) -> Result<ComponentsCanonical, ShimError> {
    let mut components: Vec<CanonicalComponent> = Vec::with_capacity(l9.components.len());
    // Sorted iteration for deterministic on-disk output (HashMap's
    // iteration order is non-deterministic by design).
    let mut keys: Vec<&String> = l9.components.keys().collect();
    keys.sort();
    for id in keys {
        let output = &l9.components[id];
        let path_label = format!("components.{id}");

        let kind = require_string_field(&output.value, "kind", &path_label)?;
        let language = require_string_field(&output.value, "language", &path_label)?;
        let lifecycle = require_string_field(&output.value, "lifecycle", &path_label)?;
        let subsystem = output
            .value
            .get("subsystem_hint")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let evidence_pointers = read_evidence_pointers(&output.value);
        let confidence_grade = output
            .value
            .get("confidence_grade")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        components.push(CanonicalComponent {
            id: id.clone(),
            kind,
            language,
            lifecycle,
            subsystem,
            evidence_pointers,
            confidence_grade,
        });
    }
    Ok(ComponentsCanonical {
        schema_version: LLM_CANONICAL_SCHEMA_VERSION,
        components,
    })
}

/// Build the `subsystems.yaml` canonical struct from `l9.subsystems`.
///
/// Required field: `purpose`. Other fields default to empty when
/// absent.
fn build_subsystems_canonical(l9: &L9Projection) -> Result<SubsystemsCanonical, ShimError> {
    let mut subsystems: Vec<CanonicalSubsystem> = Vec::with_capacity(l9.subsystems.len());
    let mut keys: Vec<&String> = l9.subsystems.keys().collect();
    keys.sort();
    for id in keys {
        let output = &l9.subsystems[id];
        let path_label = format!("subsystems.{id}");

        let purpose = require_string_field(&output.value, "purpose", &path_label)?;
        let component_ids = read_string_array(&output.value, "component_ids");
        let key_contracts = read_key_contracts(&output.value);
        let refactoring_cues = read_refactoring_cues(&output.value);
        let evidence_pointers = read_evidence_pointers(&output.value);
        let confidence_grade = output
            .value
            .get("confidence_grade")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        subsystems.push(CanonicalSubsystem {
            id: id.clone(),
            purpose,
            component_ids,
            key_contracts,
            refactoring_cues,
            evidence_pointers,
            confidence_grade,
        });
    }
    Ok(SubsystemsCanonical {
        schema_version: LLM_CANONICAL_SCHEMA_VERSION,
        subsystems,
    })
}

/// Build the `related-components.yaml` canonical struct.
///
/// Edges come from two sources: each subsystem's `internal_edges`
/// (tagged `source = "internal:<subsystem_id>"`) plus the project
/// stage's `cross_subsystem_edges` (tagged `source = "cross_subsystem"`).
///
/// Hard-fail on missing `workspace_purpose` on `l9.project` — that's
/// the canonical "did the project agent produce a coherent
/// workspace-level view" signal, and it cuts the test fixture for
/// the missing-field test path. The required-field check fires here
/// rather than in `build_components_canonical` so a missing
/// workspace_purpose surfaces via the related-components builder; the
/// test fixture (`missing_workspace_purpose_surfaces_shim_error_no_disk_residue`)
/// expects the error before any disk write.
fn build_related_components_canonical(
    l9: &L9Projection,
) -> Result<RelatedComponentsCanonical, ShimError> {
    // Project stage workspace_purpose is the canonical project-output
    // anchor; hard-fail if it's missing.
    let project = l9
        .project
        .as_ref()
        .ok_or_else(|| ShimError::MissingProjectionField {
            field: "workspace_purpose",
            path: "project".to_string(),
        })?;
    let _workspace_purpose = require_string_field(&project.value, "workspace_purpose", "project")?;

    let mut edges: Vec<CanonicalEdge> = Vec::new();

    // Internal edges from each subsystem reduce.
    let mut subsystem_keys: Vec<&String> = l9.subsystems.keys().collect();
    subsystem_keys.sort();
    for subsystem_id in subsystem_keys {
        let output = &l9.subsystems[subsystem_id];
        let source = format!("internal:{subsystem_id}");
        edges.extend(read_edges(&output.value, "internal_edges", &source));
    }

    // Cross-subsystem edges from the project stage.
    edges.extend(read_edges(
        &project.value,
        "cross_subsystem_edges",
        "cross_subsystem",
    ));

    Ok(RelatedComponentsCanonical {
        schema_version: LLM_CANONICAL_SCHEMA_VERSION,
        edges,
    })
}

/// Pull a required string field out of `value[field]` or surface a
/// `MissingProjectionField` error with `path` as the L9 sub-projection
/// label.
fn require_string_field(
    value: &Value,
    field: &'static str,
    path: &str,
) -> Result<String, ShimError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ShimError::MissingProjectionField {
            field,
            path: path.to_string(),
        })
}

/// Pull a `Vec<String>` out of `value[key]`. Returns empty when
/// missing / malformed.
fn read_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Read `value["evidence_pointers"]` into a flat list of
/// `CanonicalEvidencePointer { path }` rows. Drops any element
/// missing a `path` field.
fn read_evidence_pointers(value: &Value) -> Vec<CanonicalEvidencePointer> {
    value
        .get("evidence_pointers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("path")
                        .and_then(Value::as_str)
                        .map(|p| CanonicalEvidencePointer {
                            path: p.to_string(),
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_key_contracts(value: &Value) -> Vec<CanonicalContract> {
    value
        .get("key_contracts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(Value::as_str)?.to_string();
                    let kind = item.get("kind").and_then(Value::as_str)?.to_string();
                    Some(CanonicalContract { id, kind })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_refactoring_cues(value: &Value) -> Vec<CanonicalRefactoringCue> {
    value
        .get("refactoring_cues")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let kind = item.get("kind").and_then(Value::as_str)?.to_string();
                    let rationale = item
                        .get("rationale")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let component_ids = read_string_array(item, "component_ids");
                    Some(CanonicalRefactoringCue {
                        kind,
                        component_ids,
                        rationale,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_edges(value: &Value, key: &str, source: &str) -> Vec<CanonicalEdge> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let from = item.get("from").and_then(Value::as_str)?.to_string();
                    let to = item.get("to").and_then(Value::as_str)?.to_string();
                    let kind = item.get("kind").and_then(Value::as_str)?.to_string();
                    Some(CanonicalEdge {
                        from,
                        to,
                        kind,
                        source: source.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::AgentOutput;
    use serde_json::json;

    #[test]
    fn read_evidence_pointers_handles_missing_path() {
        let v = json!({
            "evidence_pointers": [
                { "path": "a.rs" },
                { "line_range": [1, 2] },
                { "path": "b.rs" }
            ]
        });
        let eps = read_evidence_pointers(&v);
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].path, "a.rs");
        assert_eq!(eps[1].path, "b.rs");
    }

    #[test]
    fn require_string_field_returns_field_name_on_missing() {
        let v = json!({});
        let err = require_string_field(&v, "kind", "components.x").unwrap_err();
        match err {
            ShimError::MissingProjectionField { field, path } => {
                assert_eq!(field, "kind");
                assert_eq!(path, "components.x");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn read_edges_tags_source_per_subsystem() {
        let v = json!({
            "internal_edges": [
                { "from": "a", "to": "b", "kind": "depends-on" }
            ]
        });
        let edges = read_edges(&v, "internal_edges", "internal:agents");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source, "internal:agents");
    }

    #[test]
    fn build_components_hard_fails_on_missing_kind() {
        let mut l9 = L9Projection::default();
        l9.components.insert(
            "atlas-cli".to_string(),
            AgentOutput::from_value(json!({ "language": "rust", "lifecycle": "build" })),
        );
        let err = build_components_canonical(&l9).unwrap_err();
        match err {
            ShimError::MissingProjectionField { field, path } => {
                assert_eq!(field, "kind");
                assert_eq!(path, "components.atlas-cli");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
