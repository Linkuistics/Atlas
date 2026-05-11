//! Phase 6 PR-3: per-component `subsystem` field overlay.
//!
//! Per-component `<path>/.atlas/overrides.yaml` with an
//! `overrides.subsystem: ...` block assigns the component to the
//! named subsystem, taking precedence over `subsystems.overrides.yaml`
//! at workspace root (LLM-spine recast spec §4.1 — closer-to-source
//! authoring). This file exercises three cases:
//!
//! 1. `per_component_subsystem_override_applies_when_central_silent` —
//!    no central entry, just a per-component override; the overlay
//!    creates the subsystem from scratch.
//! 2. `per_component_subsystem_override_wins_over_central_yaml` — both
//!    central and per-component entries reference the same component;
//!    per-component wins.
//! 3. `central_yaml_referencing_nonexistent_component_emits_warning` —
//!    central `members:` lists an id-form member that does not resolve
//!    to any extant component; pipeline emits the
//!    `SubsystemOverrideNonExistent` warning (PR-4 closed enumeration)
//!    and the run exits 0 under the default permissive collector.
//!
//! PR-4 retired the Phase 6 PR-3 transitional
//! `IndexConfig.warnings_buffer` + `WarningSink` adapter. Warning
//! capture now goes through the
//! [`atlas_engine::CapturingCollector`] installed on
//! [`atlas_cli::IndexConfig::override_warning_collector`].

use std::path::Path;
use std::sync::Arc;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig, IndexError, IndexSummary};
use atlas_engine::testing::LenientBackend;
use atlas_engine::{CapturingCollector, OverrideWarningCollector};
use atlas_index::{
    load_or_default_subsystems, save_subsystems_overrides_atomic, SubsystemEntry,
    SubsystemOverride, SubsystemsOverridesFile, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
};
use atlas_llm::LlmFingerprint;
use component_ontology::EvidenceGrade;
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0x6Du8; 32],
        ontology_sha: [0x6Eu8; 32],
        model_id: "phase6-pr3-backend".into(),
        backend_version: "v-phase6-pr3".into(),
    }
}

/// Captures the result of a single `run_index` invocation in a shape
/// mirroring `std::process::Output`: `status` is `Ok` on success,
/// `stderr` carries the captured warning text from the
/// `CapturingCollector`, and `_summary` is the index summary (unused
/// by current assertions but retained for forward compatibility).
struct RunOutput {
    status: Result<IndexSummary, IndexError>,
    stderr: String,
}

impl RunOutput {
    /// `true` when the pipeline returned `Ok`. Mirrors `status.success()`.
    fn success(&self) -> bool {
        self.status.is_ok()
    }
}

/// Drive `atlas index` against `root` with the lenient backend, with a
/// permissive `CapturingCollector` installed so the new
/// `SubsystemOverrideNonExistent` warning text is captured in-memory
/// instead of going through process stderr.
fn run_atlas_index(root: &Path) -> RunOutput {
    let collector: Arc<CapturingCollector> = Arc::new(CapturingCollector::new_permissive());
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config.override_warning_collector =
        Some(Arc::clone(&collector) as Arc<dyn OverrideWarningCollector>);

    let backend = LenientBackend::new(fingerprint());
    let status = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    );
    let stderr = collector.rendered();
    RunOutput { status, stderr }
}

/// Locate a subsystem by id in the resolved `subsystems.yaml`.
fn find_subsystem<'a>(entries: &'a [SubsystemEntry], id: &str) -> Option<&'a SubsystemEntry> {
    entries.iter().find(|s| s.id == id)
}

/// Write a minimal Cargo library crate at `<root>/<rel_path>`. The
/// classifier rule `rule_cargo_lib` recognises it deterministically;
/// no LLM `Classify` call fires.
fn write_cargo_lib(root: &Path, rel_path: &str) {
    let dir = root.join(rel_path);
    let name = dir.file_name().unwrap().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
}

/// Write a per-component override file at
/// `<root>/<rel_path>/.atlas/overrides.yaml` carrying
/// `overrides.subsystem: <name>`. The per-component form is `overrides.yaml`
/// (not `components.overrides.yaml`, which is the top-level form);
/// the engine's discovery walk locates it via a direct filesystem walk
/// because `.atlas/` contents are deliberately excluded from
/// `Workspace.files`.
fn write_per_component_subsystem_override(root: &Path, rel_path: &str, subsystem: &str) {
    let dir = root.join(rel_path).join(".atlas");
    std::fs::create_dir_all(&dir).unwrap();
    let yaml = format!("schema_version: 1\noverrides:\n  subsystem: {subsystem}\n");
    std::fs::write(dir.join("overrides.yaml"), yaml).unwrap();
}

// =============================================================
// Test 1: per-component override applied when central yaml silent.
// =============================================================

#[test]
fn per_component_subsystem_override_applies_when_central_silent() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_cargo_lib(root, "crate-a");
    write_per_component_subsystem_override(root, "crate-a", "alpha");

    let output = run_atlas_index(root);
    assert!(
        output.success(),
        "run_index must succeed: {:?}",
        output.status.as_ref().err()
    );

    let subsystems_file = load_or_default_subsystems(&root.join(".atlas/subsystems.yaml")).unwrap();
    let alpha = find_subsystem(&subsystems_file.subsystems, "alpha")
        .expect("alpha subsystem must exist; per-component override creates it from scratch");
    assert!(
        alpha.members.iter().any(|m| m.as_str() == "crate-a"),
        "expected crate-a in subsystem alpha; got members={:?}",
        alpha.members
    );
}

// =============================================================
// Test 2: per-component override WINS over central yaml entry.
// =============================================================

#[test]
fn per_component_subsystem_override_wins_over_central_yaml() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_cargo_lib(root, "crate-b");
    write_per_component_subsystem_override(root, "crate-b", "alpha");

    std::fs::create_dir_all(root.join(".atlas")).unwrap();
    save_subsystems_overrides_atomic(
        &root.join(".atlas/subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "beta".into(),
                members: vec!["crate-b".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "central yaml puts crate-b in beta".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    let output = run_atlas_index(root);
    assert!(
        output.success(),
        "run_index must succeed: {:?}",
        output.status.as_ref().err()
    );

    let subsystems_file = load_or_default_subsystems(&root.join(".atlas/subsystems.yaml")).unwrap();
    let alpha = find_subsystem(&subsystems_file.subsystems, "alpha")
        .expect("alpha subsystem must exist (per-component closer-to-source-wins overlay)");
    assert!(
        alpha.members.iter().any(|m| m.as_str() == "crate-b"),
        "expected crate-b in subsystem alpha (per-component wins over central); got alpha members={:?}",
        alpha.members
    );

    if let Some(beta) = find_subsystem(&subsystems_file.subsystems, "beta") {
        assert!(
            !beta.members.iter().any(|m| m.as_str() == "crate-b"),
            "expected crate-b NOT in subsystem beta after per-component overlay; got beta members={:?}",
            beta.members
        );
    }
}

// =============================================================
// Test 3: central yaml referencing a non-existent component emits
// the `SubsystemOverrideNonExistent` warning AND keeps the pipeline
// successful (PR-3 demoted the prior hard error; PR-4 routes the
// warning through the closed-enumeration collector).
// =============================================================

#[test]
fn central_yaml_referencing_nonexistent_component_emits_warning() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // One real component to keep the pipeline interesting.
    write_cargo_lib(root, "crate-c");

    std::fs::create_dir_all(root.join(".atlas")).unwrap();
    save_subsystems_overrides_atomic(
        &root.join(".atlas/subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "gamma".into(),
                members: vec!["nonexistent-component".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "user-authored entry referencing a stale or future id".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    let output = run_atlas_index(root);

    assert!(
        output.success(),
        "permissive mode: SubsystemOverrideNonExistent warning must not fail the run; \
         got error: {:?}",
        output.status.as_ref().err()
    );

    let stderr = &output.stderr;
    assert!(
        stderr.contains("nonexistent-component"),
        "expected warning mentioning the missing component id; got stderr: {stderr}"
    );
    assert!(
        stderr.contains("does not exist")
            || stderr.contains("not found")
            || stderr.contains("no extant"),
        "expected warning to indicate non-existence (substring `does not exist`, \
         `not found`, or `no extant`); got stderr: {stderr}"
    );
}
