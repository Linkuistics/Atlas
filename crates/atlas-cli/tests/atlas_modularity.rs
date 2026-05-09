//! Integration tests for `atlas modularity` (Phase 3 PR-10).
//!
//! Each test stands up a small Cargo workspace, runs `atlas index`
//! once to populate the engine cache + canonical YAMLs, then drives
//! `atlas_cli::reports::run_modularity` against the same workspace.
//! The shared-fixture pattern matches the existing
//! `subsystems_integration.rs` tests.

use std::path::Path;
use std::sync::Arc;

use atlas_cli::pipeline::build_engine_database;
use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::reports::{run_modularity, ModularityRunOptions, OutputFormat};
use atlas_cli::{run_index, IndexConfig};
use atlas_engine::testing::LenientBackend;
use atlas_index::{
    save_subsystems_overrides_atomic, SubsystemOverride, SubsystemsOverridesFile,
    SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
};
use atlas_llm::{LlmBackend, LlmFingerprint};
use atlas_reports::ComponentModularity;
use component_ontology::EvidenceGrade;
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [9u8; 32],
        ontology_sha: [10u8; 32],
        model_id: "modularity-test-backend".into(),
        backend_version: "v-test".into(),
    }
}

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

fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

/// Resolve a freshly-built engine database for the modularity flow.
/// Mirrors what `run_modularity_cmd` does internally, but with the
/// test backend wired through.
fn build_db_for_test(
    config: &IndexConfig,
) -> (atlas_engine::AtlasDatabase, Vec<std::path::PathBuf>) {
    let backend: Arc<dyn LlmBackend> = LenientBackend::new(fingerprint());
    let reporter = make_stderr_reporter(ProgressMode::Never, None);
    build_engine_database(config, backend, reporter).expect("build_engine_database succeeded")
}

fn modularity_options_for(
    config: &IndexConfig,
    roots: Vec<std::path::PathBuf>,
    no_write: bool,
) -> ModularityRunOptions {
    ModularityRunOptions {
        format: OutputFormat::Yaml,
        no_write,
        roots,
        output_dir: config.output_dir.clone(),
    }
}

// ---------------------------------------------------------------
// AC: atlas_modularity_first_run_writes_per_component_files
// ---------------------------------------------------------------

#[test]
fn atlas_modularity_first_run_writes_per_component_files() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_cargo_lib(root, "lib-a");
    write_cargo_lib(root, "lib-b");

    let config = base_config(root);
    std::fs::create_dir_all(&config.output_dir).unwrap();
    run_index(
        &config,
        LenientBackend::new(fingerprint()),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let (db, roots) = build_db_for_test(&config);
    let opts = modularity_options_for(&config, roots, false);
    let mut sink: Vec<u8> = Vec::new();
    let report = run_modularity(&db, &opts, &mut sink).unwrap();
    assert!(
        !report.per_component.is_empty(),
        "expected at least one component in the report"
    );

    for cid in report.per_component.keys() {
        // Each live component must now have a `<component>/.atlas/cache/modularity.yaml`.
        // Path resolution: lib-a / lib-b are at the workspace root.
        let component_path = root.join(cid.as_str());
        let modularity_path = component_path
            .join(".atlas")
            .join("cache")
            .join("modularity.yaml");
        assert!(
            modularity_path.exists(),
            "expected per-component modularity.yaml at {} (component {})",
            modularity_path.display(),
            cid.as_str()
        );
        let bytes = std::fs::read(&modularity_path).unwrap();
        let parsed: ComponentModularity = serde_yaml::from_slice(&bytes).unwrap();
        assert_eq!(
            parsed.history.len(),
            1,
            "first run must produce exactly one history entry for {}",
            cid.as_str()
        );
    }
}

// ---------------------------------------------------------------
// AC: atlas_modularity_second_run_with_no_changes_no_history_append
// ---------------------------------------------------------------

#[test]
fn atlas_modularity_second_run_with_no_changes_no_history_append() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_cargo_lib(root, "lib-a");

    let config = base_config(root);
    std::fs::create_dir_all(&config.output_dir).unwrap();
    run_index(
        &config,
        LenientBackend::new(fingerprint()),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    // Run #1: writes one history entry per component.
    {
        let (db, roots) = build_db_for_test(&config);
        let opts = modularity_options_for(&config, roots, false);
        let mut sink: Vec<u8> = Vec::new();
        run_modularity(&db, &opts, &mut sink).unwrap();
    }
    // Run #2: same workspace, same surfaces → fingerprint matches →
    // history must NOT grow.
    {
        let (db, roots) = build_db_for_test(&config);
        let opts = modularity_options_for(&config, roots, false);
        let mut sink: Vec<u8> = Vec::new();
        run_modularity(&db, &opts, &mut sink).unwrap();
    }

    let modularity_path = root
        .join("lib-a")
        .join(".atlas")
        .join("cache")
        .join("modularity.yaml");
    let bytes = std::fs::read(&modularity_path).unwrap();
    let parsed: ComponentModularity = serde_yaml::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed.history.len(),
        1,
        "second run with no surface change must NOT append to history; got {} entries",
        parsed.history.len()
    );
}

// ---------------------------------------------------------------
// AC: atlas_modularity_second_run_with_surface_change_appends_history
// ---------------------------------------------------------------

#[test]
fn atlas_modularity_second_run_with_surface_change_appends_history() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_cargo_lib(root, "lib-a");

    let config = base_config(root);
    std::fs::create_dir_all(&config.output_dir).unwrap();
    run_index(
        &config,
        LenientBackend::new(fingerprint()),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    // Run #1.
    {
        let (db, roots) = build_db_for_test(&config);
        let opts = modularity_options_for(&config, roots, false);
        let mut sink: Vec<u8> = Vec::new();
        run_modularity(&db, &opts, &mut sink).unwrap();
    }
    // Mutate the component's source file. The Rust surface analyser
    // produces contracts for each `pub struct ... #[derive(Serialize,
    // Deserialize)]`. Adding such a struct moves the surface
    // fingerprint and forces a new history entry. The Salsa file
    // input is fingerprinted by content_sha; re-running L0 seed via
    // `build_engine_database` picks up the change.
    std::fs::write(
        root.join("lib-a/src/lib.rs"),
        "use serde::{Deserialize, Serialize};\n\
         \n\
         #[derive(Serialize, Deserialize)]\n\
         pub struct Hello { pub name: String }\n",
    )
    .unwrap();

    // Re-run `atlas index` so the L4–L8 outputs reflect the source
    // change before modularity recomputes.
    run_index(
        &config,
        LenientBackend::new(fingerprint()),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    // Run #2 against the mutated workspace.
    {
        let (db, roots) = build_db_for_test(&config);
        let opts = modularity_options_for(&config, roots, false);
        let mut sink: Vec<u8> = Vec::new();
        run_modularity(&db, &opts, &mut sink).unwrap();
    }

    let modularity_path = root
        .join("lib-a")
        .join(".atlas")
        .join("cache")
        .join("modularity.yaml");
    let bytes = std::fs::read(&modularity_path).unwrap();
    let parsed: ComponentModularity = serde_yaml::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed.history.len(),
        2,
        "surface-change run must append to history; got {} entries",
        parsed.history.len()
    );
}

// ---------------------------------------------------------------
// AC: atlas_modularity_writes_rollup_at_top_level
// ---------------------------------------------------------------

#[test]
fn atlas_modularity_writes_rollup_at_top_level() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_cargo_lib(root, "lib-a");
    write_cargo_lib(root, "lib-b");

    let config = base_config(root);
    std::fs::create_dir_all(&config.output_dir).unwrap();

    // Author a subsystems-overrides.yaml so the rollup actually has
    // an aggregate entry to write.
    save_subsystems_overrides_atomic(
        &config.output_dir.join("subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "core-libs".into(),
                members: vec!["lib-a".into(), "lib-b".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    run_index(
        &config,
        LenientBackend::new(fingerprint()),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let (db, roots) = build_db_for_test(&config);
    let opts = modularity_options_for(&config, roots, false);
    let mut sink: Vec<u8> = Vec::new();
    let report = run_modularity(&db, &opts, &mut sink).unwrap();

    let rollup_path = config
        .output_dir
        .join("cache")
        .join("reports")
        .join("modularity-rollup.yaml");
    assert!(
        rollup_path.exists(),
        "expected rollup file at {}",
        rollup_path.display()
    );
    let bytes = std::fs::read(&rollup_path).unwrap();
    let on_disk: atlas_reports::ModularityRollup = serde_yaml::from_slice(&bytes).unwrap();
    assert_eq!(
        on_disk.subsystems.len(),
        1,
        "rollup must carry the one subsystem we authored"
    );
    assert_eq!(on_disk.subsystems[0].id, "core-libs");
    assert_eq!(report.rollup.subsystems[0].id, "core-libs");
}

// ---------------------------------------------------------------
// AC: atlas_modularity_no_write_skips_writes
// ---------------------------------------------------------------

#[test]
fn atlas_modularity_no_write_skips_writes() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_cargo_lib(root, "lib-a");

    let config = base_config(root);
    std::fs::create_dir_all(&config.output_dir).unwrap();
    run_index(
        &config,
        LenientBackend::new(fingerprint()),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let (db, roots) = build_db_for_test(&config);
    let opts = modularity_options_for(&config, roots, /*no_write=*/ true);
    let mut sink: Vec<u8> = Vec::new();
    let _report = run_modularity(&db, &opts, &mut sink).unwrap();

    let modularity_path = root
        .join("lib-a")
        .join(".atlas")
        .join("cache")
        .join("modularity.yaml");
    assert!(
        !modularity_path.exists(),
        "no-write run must NOT create per-component modularity.yaml; found {}",
        modularity_path.display()
    );
    let rollup_path = config
        .output_dir
        .join("cache")
        .join("reports")
        .join("modularity-rollup.yaml");
    assert!(
        !rollup_path.exists(),
        "no-write run must NOT create the rollup file; found {}",
        rollup_path.display()
    );
}
