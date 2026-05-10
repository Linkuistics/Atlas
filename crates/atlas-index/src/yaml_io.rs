//! Load / save helpers for the five Atlas-owned YAMLs.
//!
//! Five pairs of functions — `load_components` / `save_components_atomic`
//! and analogues for overrides, externals, subsystems, and subsystems
//! overrides. `load_or_default_*` returns the schema-default when the
//! file is absent; the strict `load_*` errors out if absent.
//!
//! Schema-version mismatch is a hard error. The error message differs
//! between generated and user-authored files: generated files
//! (`components.yaml`, `external-components.yaml`, `subsystems.yaml`)
//! tell the user to delete and re-run the tool; user-authored files
//! (`components.overrides.yaml`, `subsystems.overrides.yaml`) point at
//! the migration docs because deleting hand-authored content is never
//! the right answer.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::schema::{
    ComponentsFile, ExternalsFile, OverridesFile, SubsystemsFile, SubsystemsOverridesFile,
    COMPONENTS_SCHEMA_VERSION, EXTERNALS_SCHEMA_VERSION, OVERRIDES_SCHEMA_VERSION,
    SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION, SUBSYSTEMS_SCHEMA_VERSION,
};

pub fn load_components(path: &Path) -> Result<ComponentsFile> {
    let content = read_to_string(path)?;
    let file: ComponentsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {} as components.yaml", path.display()))?;
    require_schema_version(
        file.schema_version,
        COMPONENTS_SCHEMA_VERSION,
        path,
        FileFlavour::Generated {
            tool_step: "atlas index",
        },
    )?;
    Ok(file)
}

pub fn load_or_default_components(path: &Path) -> Result<ComponentsFile> {
    if !path.exists() {
        return Ok(ComponentsFile::default());
    }
    load_components(path)
}

pub fn save_components_atomic(path: &Path, file: &ComponentsFile) -> Result<()> {
    let yaml = serde_yaml::to_string(file).context("failed to serialise components.yaml")?;
    write_atomic(path, yaml.as_bytes())
}

pub fn load_overrides(path: &Path) -> Result<OverridesFile> {
    let content = read_to_string(path)?;
    let file: OverridesFile = serde_yaml::from_str(&content).with_context(|| {
        format!(
            "failed to parse {} as components.overrides.yaml",
            path.display()
        )
    })?;
    require_schema_version(
        file.schema_version,
        OVERRIDES_SCHEMA_VERSION,
        path,
        FileFlavour::UserAuthored,
    )?;
    Ok(file)
}

pub fn load_or_default_overrides(path: &Path) -> Result<OverridesFile> {
    if !path.exists() {
        return Ok(OverridesFile::default());
    }
    load_overrides(path)
}

pub fn save_overrides_atomic(path: &Path, file: &OverridesFile) -> Result<()> {
    let yaml =
        serde_yaml::to_string(file).context("failed to serialise components.overrides.yaml")?;
    write_atomic(path, yaml.as_bytes())
}

pub fn load_externals(path: &Path) -> Result<ExternalsFile> {
    let content = read_to_string(path)?;
    let file: ExternalsFile = serde_yaml::from_str(&content).with_context(|| {
        format!(
            "failed to parse {} as external-components.yaml",
            path.display()
        )
    })?;
    require_schema_version(
        file.schema_version,
        EXTERNALS_SCHEMA_VERSION,
        path,
        FileFlavour::Generated {
            tool_step: "atlas index",
        },
    )?;
    Ok(file)
}

pub fn load_or_default_externals(path: &Path) -> Result<ExternalsFile> {
    if !path.exists() {
        return Ok(ExternalsFile::default());
    }
    load_externals(path)
}

pub fn save_externals_atomic(path: &Path, file: &ExternalsFile) -> Result<()> {
    let yaml =
        serde_yaml::to_string(file).context("failed to serialise external-components.yaml")?;
    write_atomic(path, yaml.as_bytes())
}

pub fn load_subsystems_overrides(path: &Path) -> Result<SubsystemsOverridesFile> {
    let content = read_to_string(path)?;
    let file: SubsystemsOverridesFile = serde_yaml::from_str(&content).with_context(|| {
        format!(
            "failed to parse {} as subsystems.overrides.yaml",
            path.display()
        )
    })?;
    require_schema_version(
        file.schema_version,
        SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
        path,
        FileFlavour::UserAuthored,
    )?;
    Ok(file)
}

pub fn load_or_default_subsystems_overrides(path: &Path) -> Result<SubsystemsOverridesFile> {
    if !path.exists() {
        return Ok(SubsystemsOverridesFile::default());
    }
    load_subsystems_overrides(path)
}

pub fn save_subsystems_overrides_atomic(path: &Path, file: &SubsystemsOverridesFile) -> Result<()> {
    let yaml =
        serde_yaml::to_string(file).context("failed to serialise subsystems.overrides.yaml")?;
    write_atomic(path, yaml.as_bytes())
}

pub fn load_subsystems(path: &Path) -> Result<SubsystemsFile> {
    let content = read_to_string(path)?;
    let file: SubsystemsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {} as subsystems.yaml", path.display()))?;
    require_schema_version(
        file.schema_version,
        SUBSYSTEMS_SCHEMA_VERSION,
        path,
        FileFlavour::Generated {
            tool_step: "atlas index",
        },
    )?;
    Ok(file)
}

pub fn load_or_default_subsystems(path: &Path) -> Result<SubsystemsFile> {
    if !path.exists() {
        return Ok(SubsystemsFile::default());
    }
    load_subsystems(path)
}

pub fn save_subsystems_atomic(path: &Path, file: &SubsystemsFile) -> Result<()> {
    let yaml = serde_yaml::to_string(file).context("failed to serialise subsystems.yaml")?;
    write_atomic(path, yaml.as_bytes())
}

fn read_to_string(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

enum FileFlavour {
    /// Regenerating the file is the normal upgrade path.
    Generated { tool_step: &'static str },
    /// Regenerating would destroy user-entered pins.
    UserAuthored,
}

fn require_schema_version(
    actual: u32,
    expected: u32,
    path: &Path,
    flavour: FileFlavour,
) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    match flavour {
        FileFlavour::Generated { tool_step } => bail!(
            "{} has schema_version {} but atlas-index expects {}. \
             This file is regenerated by the producing tool; the supported \
             upgrade path is to delete the file and re-run `{}`.",
            path.display(),
            actual,
            expected,
            tool_step
        ),
        FileFlavour::UserAuthored => bail!(
            "{} has schema_version {} but atlas-index expects {}. \
             This file is user-authored and must be migrated by hand — \
             see docs/overrides-migration.md (once published) for the \
             schema delta. Do NOT delete it; that would erase all pins.",
            path.display(),
            actual,
            expected
        ),
    }
}

/// Write via temp-file-then-rename so a crash mid-write cannot leave a
/// half-serialised file at `path`. Temp file sits alongside the target
/// so the rename is on the same filesystem.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name component", path.display()))?
        .to_string_lossy()
        .into_owned();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use component_ontology::{ComponentId, EvidenceGrade, LifecycleScope};
    use tempfile::TempDir;

    use super::super::schema::{
        AlwaysTrue, CacheFingerprints, ComponentEntry, ComponentFieldOverrides, ComponentsFile,
        DocAnchor, EdgeAdd, EdgeSuppress, ExternalEntry, ExternalsFile, OverridesFile, PathSegment,
        PinValue, SubsystemOverride, SubsystemsFile, SubsystemsOverridesFile,
        COMPONENTS_SCHEMA_VERSION, EXTERNALS_SCHEMA_VERSION, OVERRIDES_SCHEMA_VERSION,
        SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION, SUBSYSTEMS_SCHEMA_VERSION,
    };
    use super::*;

    fn sample_component_entry() -> ComponentEntry {
        ComponentEntry {
            id: ComponentId::parse("atlas").unwrap(),
            parent: None,
            kind: "workspace".into(),
            lifecycle_roles: vec![LifecycleScope::Build],
            languages: BTreeSet::from(["rust".to_string()]),
            build_system: Some("cargo".into()),
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("."),
                content_sha: "deadbeef".into(),
            }],
            manifests: vec![PathBuf::from("Cargo.toml")],
            doc_anchors: vec![DocAnchor {
                path: PathBuf::from("README.md"),
                heading: "Atlas".into(),
            }],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["Cargo.toml:[workspace]".into()],
            rationale: "Cargo.toml declares a workspace".into(),
            deleted: false,
        }
    }

    fn sample_components_file() -> ComponentsFile {
        ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            roots: vec![PathBuf::from("/tmp/target")],
            generated_at: "2026-04-24T00:00:00Z".into(),
            cache_fingerprints: CacheFingerprints {
                ontology_sha: "0123".into(),
                prompt_shas: BTreeMap::from([("classify".to_string(), "4567".to_string())]),
                model_id: "claude-opus-4-7".into(),
                backend_version: "claude-cli 1.0.0".into(),
                analyzer_registry_sha: "89ab".into(),
            },
            components: vec![sample_component_entry()],
        }
    }

    fn sample_overrides_file() -> OverridesFile {
        let mut pins: BTreeMap<ComponentId, BTreeMap<String, PinValue>> = BTreeMap::new();
        let mut component_pins: BTreeMap<String, PinValue> = BTreeMap::new();
        component_pins.insert(
            "role".into(),
            PinValue::Value {
                value: "spec".into(),
                reason: Some("hand-curated docs repo".into()),
            },
        );
        component_pins.insert(
            "deleted".into(),
            PinValue::Suppress {
                suppress: AlwaysTrue,
            },
        );
        pins.insert(
            ComponentId::parse("atlas/deprecated-crate").unwrap(),
            component_pins,
        );
        OverridesFile {
            schema_version: OVERRIDES_SCHEMA_VERSION,
            pins,
            additions: vec![sample_component_entry()],
            edges_add: vec![EdgeAdd {
                kind: "bundled-into".into(),
                from: "atlas/atlas-engine".into(),
                to: "atlas/atlas-cli".into(),
                reason: "manual annotation - shipped together by the release script".into(),
            }],
            edges_suppress: vec![EdgeSuppress {
                kind: "depends-on".into(),
                from: "atlas/atlas-engine".into(),
                to: "atlas/atlas-cli".into(),
                reason: "false positive - dev-only path".into(),
            }],
            field_overrides: ComponentFieldOverrides {
                language: Some("rust".into()),
                kind: Some("rust-library".into()),
                lifecycle: Some("build-time".into()),
                subsystem: Some("ravel-lite/runtime".into()),
            },
        }
    }

    fn sample_externals_file() -> ExternalsFile {
        ExternalsFile {
            schema_version: EXTERNALS_SCHEMA_VERSION,
            externals: vec![ExternalEntry {
                id: "crate:serde".into(),
                kind: "external".into(),
                language: Some("rust".into()),
                purl: Some("pkg:cargo/serde@1".into()),
                homepage: None,
                url: None,
                discovered_from: vec!["Cargo.toml".into()],
                evidence_grade: EvidenceGrade::Strong,
            }],
        }
    }

    #[test]
    fn components_save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.yaml");
        let original = sample_components_file();
        save_components_atomic(&path, &original).unwrap();
        let loaded = load_components(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn overrides_save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.overrides.yaml");
        let original = sample_overrides_file();
        save_overrides_atomic(&path, &original).unwrap();
        let loaded = load_overrides(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn overrides_load_accepts_natural_inline_suppress_children_form() {
        // Reported user file shape: `suppress_children: [<id>, ...]` written
        // directly under a pin entry, alongside `kind`, `language`, `role` pins.
        // Before custom (de)serialise impls landed on PinValue, this produced
        // "data did not match any variant of untagged enum PinValue".
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.overrides.yaml");
        std::fs::write(
            &path,
            "schema_version: 1\n\
             pins:\n  \
               connector-lib:\n    \
                 kind: { value: rust-library, reason: \"top-level crate\" }\n    \
                 language: { value: rust }\n    \
                 role: { value: connector-core }\n    \
                 suppress_children: [connector-lib/formats]\n",
        )
        .unwrap();
        let loaded = load_overrides(&path).unwrap();
        let connector_pins = loaded
            .pins
            .get(&ComponentId::parse("connector-lib").unwrap())
            .expect("pin entry present");
        match connector_pins.get("suppress_children") {
            Some(PinValue::SuppressChildren { suppress_children }) => {
                assert_eq!(
                    suppress_children,
                    &vec![ComponentId::parse("connector-lib/formats").unwrap()]
                );
            }
            other => panic!("expected SuppressChildren, got {other:?}"),
        }
    }

    #[test]
    fn externals_save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("external-components.yaml");
        let original = sample_externals_file();
        save_externals_atomic(&path, &original).unwrap();
        let loaded = load_externals(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn load_or_default_components_returns_default_when_missing() {
        let tmp = TempDir::new().unwrap();
        let file = load_or_default_components(&tmp.path().join("components.yaml")).unwrap();
        assert_eq!(file.schema_version, COMPONENTS_SCHEMA_VERSION);
        assert!(file.components.is_empty());
    }

    #[test]
    fn load_or_default_overrides_returns_default_when_missing() {
        let tmp = TempDir::new().unwrap();
        let file =
            load_or_default_overrides(&tmp.path().join("components.overrides.yaml")).unwrap();
        assert_eq!(file.schema_version, OVERRIDES_SCHEMA_VERSION);
        assert!(file.pins.is_empty());
    }

    #[test]
    fn load_or_default_externals_returns_default_when_missing() {
        let tmp = TempDir::new().unwrap();
        let file = load_or_default_externals(&tmp.path().join("external-components.yaml")).unwrap();
        assert_eq!(file.schema_version, EXTERNALS_SCHEMA_VERSION);
        assert!(file.externals.is_empty());
    }

    #[test]
    fn components_load_rejects_wrong_schema_version_with_regenerate_hint() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.yaml");
        std::fs::write(
            &path,
            "schema_version: 99\nroots: [/tmp]\ngenerated_at: ''\ncache_fingerprints: {ontology_sha: '', model_id: '', backend_version: ''}\ncomponents: []\n",
        )
        .unwrap();
        let err = load_components(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "msg: {msg}");
        assert!(
            msg.contains("atlas index"),
            "msg must point at the tool step: {msg}"
        );
    }

    #[test]
    fn externals_load_rejects_wrong_schema_version_with_regenerate_hint() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("external-components.yaml");
        std::fs::write(&path, "schema_version: 99\nexternals: []\n").unwrap();
        let err = load_externals(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("atlas index"), "msg: {msg}");
    }

    #[test]
    fn overrides_load_rejects_wrong_schema_version_and_warns_against_deletion() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.overrides.yaml");
        std::fs::write(&path, "schema_version: 99\npins: {}\nadditions: []\n").unwrap();
        let err = load_overrides(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("user-authored"), "msg: {msg}");
        assert!(
            msg.contains("not delete") || msg.contains("Do NOT delete"),
            "message must warn against deletion: {msg}"
        );
    }

    #[test]
    fn overrides_load_rejects_invalid_component_id_as_pin_key() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.overrides.yaml");
        std::fs::write(
            &path,
            "schema_version: 1\npins:\n  Bad/Name:\n    kind: {value: library}\n",
        )
        .unwrap();
        let err = load_overrides(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("component id"),
            "expected component id error, got: {msg}"
        );
    }

    #[test]
    fn overrides_load_rejects_invalid_component_id_in_suppress_children() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.overrides.yaml");
        std::fs::write(
            &path,
            "schema_version: 1\npins:\n  my-crate:\n    suppress_children: [Bad/Name]\n",
        )
        .unwrap();
        let err = load_overrides(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("component id"),
            "expected component id error, got: {msg}"
        );
    }

    #[test]
    fn save_atomic_never_leaves_temp_file_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.yaml");
        save_components_atomic(&path, &ComponentsFile::default()).unwrap();
        let leftover: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(
            leftover.is_empty(),
            "atomic save left a temp file: {:?}",
            leftover.iter().map(|e| e.file_name()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn strict_load_errors_on_missing_file() {
        let tmp = TempDir::new().unwrap();
        let err = load_components(&tmp.path().join("nope.yaml")).unwrap_err();
        assert!(format!("{err:#}").contains("failed to read"));
    }

    fn sample_subsystems_overrides_file() -> SubsystemsOverridesFile {
        SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["services/auth/*".into(), "identity-core".into()],
                role: Some("identity".into()),
                lifecycle_roles: vec![],
                rationale: "x".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        }
    }

    fn sample_subsystems_file() -> SubsystemsFile {
        SubsystemsFile {
            schema_version: SUBSYSTEMS_SCHEMA_VERSION,
            generated_at: "2026-05-01T00:00:00Z".into(),
            subsystems: vec![],
        }
    }

    #[test]
    fn subsystems_overrides_save_then_load_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subsystems.overrides.yaml");
        let original = sample_subsystems_overrides_file();
        save_subsystems_overrides_atomic(&path, &original).unwrap();
        let loaded = load_subsystems_overrides(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn subsystems_save_then_load_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subsystems.yaml");
        let original = sample_subsystems_file();
        save_subsystems_atomic(&path, &original).unwrap();
        let loaded = load_subsystems(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn load_or_default_subsystems_overrides_returns_default_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subsystems.overrides.yaml");
        let loaded = load_or_default_subsystems_overrides(&path).unwrap();
        assert_eq!(loaded, SubsystemsOverridesFile::default());
    }

    #[test]
    fn load_or_default_subsystems_returns_default_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subsystems.yaml");
        let loaded = load_or_default_subsystems(&path).unwrap();
        assert_eq!(loaded, SubsystemsFile::default());
    }

    #[test]
    fn subsystems_overrides_load_rejects_wrong_schema_version_user_authored() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subsystems.overrides.yaml");
        std::fs::write(&path, "schema_version: 999\nsubsystems: []\n").unwrap();
        let err = load_subsystems_overrides(&path).unwrap_err().to_string();
        assert!(
            err.contains("user-authored") && err.contains("Do NOT delete"),
            "expected user-authored migration hint, got: {err}"
        );
    }

    #[test]
    fn subsystems_load_rejects_wrong_schema_version_with_regenerate_hint() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subsystems.yaml");
        std::fs::write(
            &path,
            "schema_version: 999\ngenerated_at: ''\nsubsystems: []\n",
        )
        .unwrap();
        let err = load_subsystems(&path).unwrap_err().to_string();
        assert!(
            err.contains("delete the file") && err.contains("atlas index"),
            "expected regenerate hint, got: {err}"
        );
    }

    // -----------------------------------------------------------------
    // PR-6: edges_add / edges_suppress + per-component field overrides
    // (design §5.5).
    // -----------------------------------------------------------------

    #[test]
    fn edges_add_round_trip() {
        // serialise a single EdgeAdd then round-trip — every field must
        // survive intact, including the (required) `reason`.
        let original = EdgeAdd {
            kind: "bundled-into".into(),
            from: "billing-core".into(),
            to: "billing-orchestration".into(),
            reason: "release script ships these together".into(),
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: EdgeAdd = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn edges_suppress_round_trip() {
        let original = EdgeSuppress {
            kind: "depends-on".into(),
            from: "billing-core".into(),
            to: "auth-core".into(),
            reason: "false positive — dev-only path".into(),
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: EdgeSuppress = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn edges_add_missing_reason_rejects() {
        // The design spec says `reason` is required on every edges_add
        // entry. The Rust type leaves the `reason` field without
        // `#[serde(default)]`, so a missing reason must produce a
        // deserialise error rather than silently materialising an
        // empty string.
        let yaml =
            "schema_version: 1\nedges_add:\n  - kind: bundled-into\n    from: a\n    to: b\n";
        let err = serde_yaml::from_str::<OverridesFile>(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("reason") || msg.contains("missing field"),
            "expected missing-field error mentioning reason, got: {msg}"
        );
    }

    #[test]
    fn edges_suppress_missing_reason_rejects() {
        let yaml =
            "schema_version: 1\nedges_suppress:\n  - kind: depends-on\n    from: a\n    to: b\n";
        let err = serde_yaml::from_str::<OverridesFile>(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("reason") || msg.contains("missing field"),
            "expected missing-field error mentioning reason, got: {msg}"
        );
    }

    #[test]
    fn field_overrides_round_trip() {
        // Per-component overrides round-trip through the `overrides:`
        // YAML key (not `field_overrides:`), matching the design's
        // example shape.
        let yaml = "schema_version: 1\noverrides:\n  language: rust\n  kind: rust-library\n  \
                    lifecycle: build-time\n  subsystem: ravel-lite/runtime\n";
        let parsed: OverridesFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.field_overrides.language.as_deref(), Some("rust"));
        assert_eq!(parsed.field_overrides.kind.as_deref(), Some("rust-library"));
        assert_eq!(
            parsed.field_overrides.lifecycle.as_deref(),
            Some("build-time")
        );
        assert_eq!(
            parsed.field_overrides.subsystem.as_deref(),
            Some("ravel-lite/runtime")
        );
        let re_emitted = serde_yaml::to_string(&parsed).unwrap();
        // Re-emit uses the canonical `overrides:` key (rename), and the
        // inner fields preserve the four authored values.
        assert!(
            re_emitted.contains("overrides:"),
            "round-trip must emit `overrides:` wrapper, got:\n{re_emitted}"
        );
        assert!(re_emitted.contains("language: rust"));
        assert!(re_emitted.contains("kind: rust-library"));
        assert!(re_emitted.contains("lifecycle: build-time"));
        assert!(re_emitted.contains("subsystem: ravel-lite/runtime"));
    }

    #[test]
    fn field_overrides_default_all_none() {
        let f = OverridesFile::default();
        assert!(f.field_overrides.language.is_none());
        assert!(f.field_overrides.kind.is_none());
        assert!(f.field_overrides.lifecycle.is_none());
        assert!(f.field_overrides.subsystem.is_none());
        assert!(f.field_overrides.is_empty());
    }

    #[test]
    fn field_overrides_empty_block_omits_overrides_key_in_yaml() {
        let f = OverridesFile {
            field_overrides: ComponentFieldOverrides::default(),
            ..OverridesFile::default()
        };
        let yaml = serde_yaml::to_string(&f).unwrap();
        assert!(
            !yaml.contains("overrides:"),
            "empty field_overrides must skip the `overrides:` key, got:\n{yaml}"
        );
    }

    #[test]
    fn overrides_file_with_new_fields_round_trips_through_atomic_save() {
        // Full-shape round trip through the on-disk saver: confirms
        // that the extended OverridesFile (pins + additions +
        // edges_add + edges_suppress + field_overrides) preserves
        // every field across serde + atomic-write + read-back.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("components.overrides.yaml");
        let original = sample_overrides_file();
        save_overrides_atomic(&path, &original).unwrap();
        let loaded = load_overrides(&path).unwrap();
        assert_eq!(loaded, original);
        assert_eq!(loaded.edges_add.len(), 1);
        assert_eq!(loaded.edges_add[0].reason, original.edges_add[0].reason);
        assert_eq!(loaded.edges_suppress.len(), 1);
        assert_eq!(
            loaded.field_overrides.language.as_deref(),
            Some("rust"),
            "field_overrides.language must round-trip through atomic save"
        );
    }
}
