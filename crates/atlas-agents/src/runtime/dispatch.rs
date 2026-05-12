//! Workspace → subsystem → component partitioning.
//!
//! **PR-4: deterministic-only.** The runtime requires both
//! `subsystems.overrides.yaml` and `components.overrides.yaml` to be
//! present at the workspace root. PR-5 relaxes this — when an override
//! file is absent, an LLM-driven dispatch agent fills the gap (Lane A
//! validation gates that flow). No LLM calls happen here in PR-4.
//!
//! Both files are parsed via `serde_yaml::from_str` with
//! `#[serde(deny_unknown_fields)]` shape, so typos surface as parse
//! errors rather than silent drops. The parse result is Lane-A
//! validated for structural sanity (non-empty id, etc.) before being
//! handed to `AgentRuntime`.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::AgentError;

/// One subsystem partition resolved from `subsystems.overrides.yaml`.
/// Minimal PR-4 shape: id + members. PR-5 may extend with additional
/// metadata (role, lifecycle_roles, evidence_grade) — those live in
/// `SubsystemsOverridesFile` already; PR-4 carries only what
/// `run_iteration` needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemPartition {
    /// Stable id of this subsystem (e.g. `"agents"`).
    pub id: String,
    /// Members — each entry is a component id. Glob expansion (the
    /// `*` form) is **not** performed here in PR-4; the override file
    /// must list resolved ids. PR-5 widens this to deferred resolution.
    pub members: Vec<String>,
}

/// One component partition resolved from `components.overrides.yaml`,
/// after subsystem-membership filtering. Minimal PR-4 shape: id +
/// subsystem id (back-pointer for diagnostics). PR-5 may extend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentPartition {
    /// Stable component id.
    pub id: String,
    /// Subsystem the component belongs to (the dispatcher decided).
    pub subsystem_id: String,
    /// Optional field overrides parsed from the per-component override
    /// file (Phase 6 PR-3 / recast §4.3). PR-4 carries them through;
    /// PR-5 will route them into the Classify-stage prompt context.
    #[serde(default)]
    pub field_overrides: ComponentFieldOverrides,
}

/// Subset of the `OverridesFile.field_overrides` shape that PR-4
/// needs to propagate. Mirrors `atlas_index::schema::ComponentFieldOverrides`
/// without taking a dependency on that type's `#[serde(deny_unknown_fields)]`
/// future evolution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentFieldOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
}

/// On-disk shape for `subsystems.overrides.yaml`. Uses
/// `#[serde(deny_unknown_fields)]` at both levels so typos in either
/// the top-level key list or any inner subsystem entry fail loudly at
/// parse time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubsystemsOverrideFile {
    /// Schema version. PR-4 accepts `1` (today's only value); a future
    /// reader can match on this for upgrade paths.
    pub schema_version: u32,
    /// Hand-authored subsystems.
    #[serde(default)]
    pub subsystems: Vec<SubsystemOverrideEntry>,
}

/// One subsystem entry inside `subsystems.overrides.yaml`. PR-4 reads
/// only `id` + `members`; the trailing fields exist to keep the parse
/// total against the wider production schema (which carries
/// `role`/`lifecycle_roles`/`rationale`/`evidence_grade`/`evidence_fields`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubsystemOverrideEntry {
    pub id: String,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub evidence_grade: Option<String>,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    #[serde(default)]
    pub lifecycle_roles: Vec<String>,
}

/// On-disk shape for `components.overrides.yaml`. PR-4 reads a narrow
/// subset of the wider production schema (the engine's
/// `OverridesFile` in `atlas-index`). The PR-4 shape declares only
/// the fields the runtime acts on; future fields go through
/// `extra: serde_yaml::Mapping` (commented out for now — strict-deny
/// catches typos).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentsOverrideFile {
    pub schema_version: u32,
    /// Per-component entries. The map key is the component id.
    #[serde(default)]
    pub components: std::collections::BTreeMap<String, ComponentOverrideEntry>,
}

/// One component entry inside `components.overrides.yaml`. PR-4 reads
/// only `subsystem` + the `overrides:` block (field overlays).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentOverrideEntry {
    /// The component's subsystem assignment. Optional — components
    /// without a subsystem entry are assigned to a synthetic
    /// `"_unscoped"` partition.
    #[serde(default)]
    pub subsystem: Option<String>,
    /// Phase 6 PR-3 / recast §4.3 field overlays. Carried under the
    /// YAML key `overrides:` to mirror the design example.
    #[serde(default, rename = "overrides")]
    pub field_overrides: ComponentFieldOverrides,
}

/// PR-4: load + validate `subsystems.overrides.yaml`.
///
/// Returns one `SubsystemPartition` per entry. If the file is absent,
/// returns `AgentError::OverrideRequired` — PR-5 will relax this to
/// an LLM dispatch agent that fills the gap.
pub async fn dispatch_subsystems(
    workspace_root: &Path,
) -> Result<Vec<SubsystemPartition>, AgentError> {
    let path = workspace_root.join("subsystems.overrides.yaml");
    if !path.exists() {
        return Err(AgentError::OverrideRequired(
            "subsystems.overrides.yaml is mandatory in PR-4 (PR-5 relaxes this)".into(),
        ));
    }
    let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
        AgentError::OverrideRequired(format!("failed to read {}: {e}", path.display()))
    })?;
    let parsed: SubsystemsOverrideFile = serde_yaml::from_str(&text).map_err(|e| {
        AgentError::OverrideRequired(format!("failed to parse {}: {e}", path.display()))
    })?;

    // Lane-A-style structural validation: ids are non-empty, ids are
    // unique. (Full Lane A runs on LLM output; the override file
    // sanity check is small enough to inline here.)
    let mut ids = BTreeSet::new();
    let mut partitions = Vec::with_capacity(parsed.subsystems.len());
    for entry in parsed.subsystems {
        if entry.id.is_empty() {
            return Err(AgentError::OverrideRequired(format!(
                "{}: subsystem entry with empty id",
                path.display()
            )));
        }
        if !ids.insert(entry.id.clone()) {
            return Err(AgentError::OverrideRequired(format!(
                "{}: duplicate subsystem id `{}`",
                path.display(),
                entry.id
            )));
        }
        partitions.push(SubsystemPartition {
            id: entry.id,
            members: entry.members,
        });
    }
    Ok(partitions)
}

/// PR-4: load `components.overrides.yaml` + filter to the given
/// subsystem.
///
/// Returns one `ComponentPartition` per component the override file
/// declares under `subsystem.id`. Phase 6 PR-3 subsystem field
/// overlays (per recast §4.3) are preserved on each
/// `ComponentPartition.field_overrides` for the Classify-stage
/// prompt context.
pub async fn dispatch_components(
    workspace_root: &Path,
    subsystem: &SubsystemPartition,
) -> Result<Vec<ComponentPartition>, AgentError> {
    let path = workspace_root.join("components.overrides.yaml");
    if !path.exists() {
        return Err(AgentError::OverrideRequired(
            "components.overrides.yaml is mandatory in PR-4 (PR-5 relaxes this)".into(),
        ));
    }
    let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
        AgentError::OverrideRequired(format!("failed to read {}: {e}", path.display()))
    })?;
    let parsed: ComponentsOverrideFile = serde_yaml::from_str(&text).map_err(|e| {
        AgentError::OverrideRequired(format!("failed to parse {}: {e}", path.display()))
    })?;

    let members: BTreeSet<&str> = subsystem.members.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for (id, entry) in parsed.components {
        // Membership rule: a component is in this subsystem if EITHER
        // the subsystems override file lists its id in `members`, OR
        // the component's own `subsystem:` override pins it here.
        let assigned_by_override = entry
            .subsystem
            .as_deref()
            .map(|s| s == subsystem.id)
            .unwrap_or(false);
        let assigned_by_members = members.contains(id.as_str());
        if !(assigned_by_override || assigned_by_members) {
            continue;
        }
        out.push(ComponentPartition {
            id: id.clone(),
            subsystem_id: subsystem.id.clone(),
            field_overrides: entry.field_overrides,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).expect("write fixture");
    }

    #[tokio::test]
    async fn dispatch_subsystems_returns_override_required_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = dispatch_subsystems(dir.path()).await.unwrap_err();
        assert!(matches!(err, AgentError::OverrideRequired(_)));
    }

    #[tokio::test]
    async fn dispatch_subsystems_parses_minimal_file() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "subsystems.overrides.yaml",
            "schema_version: 1\nsubsystems:\n  - id: agents\n    members: [foo, bar]\n",
        );
        let partitions = dispatch_subsystems(dir.path()).await.unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].id, "agents");
        assert_eq!(partitions[0].members, vec!["foo", "bar"]);
    }

    #[tokio::test]
    async fn dispatch_subsystems_rejects_duplicate_ids() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "subsystems.overrides.yaml",
            "schema_version: 1\nsubsystems:\n  - id: a\n    members: []\n  - id: a\n    members: []\n",
        );
        let err = dispatch_subsystems(dir.path()).await.unwrap_err();
        assert!(matches!(err, AgentError::OverrideRequired(ref msg) if msg.contains("duplicate")));
    }

    #[tokio::test]
    async fn dispatch_subsystems_rejects_empty_id() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "subsystems.overrides.yaml",
            "schema_version: 1\nsubsystems:\n  - id: ''\n    members: []\n",
        );
        let err = dispatch_subsystems(dir.path()).await.unwrap_err();
        assert!(matches!(err, AgentError::OverrideRequired(ref msg) if msg.contains("empty id")));
    }

    #[tokio::test]
    async fn dispatch_subsystems_rejects_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "subsystems.overrides.yaml",
            "schema_version: 1\nsubsystems:\n  - id: a\n    members: []\n    typo_field: 1\n",
        );
        let err = dispatch_subsystems(dir.path()).await.unwrap_err();
        assert!(matches!(err, AgentError::OverrideRequired(ref msg) if msg.contains("typo_field")));
    }

    #[tokio::test]
    async fn dispatch_components_filters_to_subsystem_members() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "components.overrides.yaml",
            "schema_version: 1\ncomponents:\n  foo:\n    subsystem: agents\n  bar:\n    subsystem: cli\n",
        );
        let subsystem = SubsystemPartition {
            id: "agents".to_string(),
            members: vec!["foo".to_string()],
        };
        let parts = dispatch_components(dir.path(), &subsystem).await.unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].id, "foo");
        assert_eq!(parts[0].subsystem_id, "agents");
    }

    #[tokio::test]
    async fn dispatch_components_honours_field_overrides() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "components.overrides.yaml",
            "schema_version: 1\ncomponents:\n  foo:\n    subsystem: agents\n    overrides:\n      kind: rust-library\n",
        );
        let subsystem = SubsystemPartition {
            id: "agents".to_string(),
            members: vec!["foo".to_string()],
        };
        let parts = dispatch_components(dir.path(), &subsystem).await.unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0].field_overrides.kind.as_deref(),
            Some("rust-library")
        );
    }
}
