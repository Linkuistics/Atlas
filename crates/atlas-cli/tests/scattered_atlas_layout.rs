//! PR-6 acceptance test: `atlas index` emits a per-component
//! `<component-path>/.atlas/component.yaml` for every live component
//! in addition to the top-level `<output>/.atlas/cache/components.yaml`.
//!
//! ## What this test asserts
//!
//! 1. Every non-deleted component in the top-level `components.yaml`
//!    has a corresponding `.atlas/component.yaml` on disk under its
//!    source path.
//! 2. The per-component file's `component` field equals the
//!    component's slot in the top-level `components.yaml` byte-for-
//!    byte (modulo serialisation). This is the §4.6 "data co-locates
//!    with source" invariant — the per-component projection IS the
//!    component's record, just stored next to its source.
//! 3. The envelope fields (`schema_version`, `surfaces_path`,
//!    `overrides_path`, `analyser_id`, `analyser_version`,
//!    `fingerprint`) are present and non-empty.

use std::path::Path;
use std::sync::{Arc, Mutex};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_index::{
    load_or_default_components, ComponentsFile, PerComponentFile, PER_COMPONENT_SCHEMA_VERSION,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::{json, Value};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [7u8; 32],
        ontology_sha: [11u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v-pr6".into(),
    }
}

/// Returns canned defaults so the pipeline can complete without the
/// per-component scattering work depending on a real LLM. Mirrors
/// `pipeline_integration::LenientBackend` in shape.
struct LenientBackend {
    fingerprint: LlmFingerprint,
    call_log: Mutex<Vec<PromptId>>,
}

impl LenientBackend {
    fn new() -> Arc<Self> {
        Arc::new(LenientBackend {
            fingerprint: fingerprint(),
            call_log: Mutex::new(Vec::new()),
        })
    }
}

impl LlmBackend for LenientBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call_log.lock().unwrap().push(req.prompt_template);
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "default lenient",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({"purpose": "stub", "notes": ""}),
            PromptId::Stage2Edges => json!([]),
            PromptId::Subcarve => json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "policy declined",
            }),
        })
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

fn write_lib(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
}

fn write_cli(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{name}\"\npath = \"src/main.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
}

#[test]
fn every_component_has_a_per_component_atlas_file_matching_the_top_level_projection() {
    let tmp = TempDir::new().unwrap();
    write_lib(tmp.path(), "alpha-lib");
    write_cli(tmp.path(), "beta-cli");

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds");
    assert!(summary.outputs_written, "outputs must be written");

    // PR-4 (Phase 3): top-level components.yaml moved to cache/.
    let top_level: ComponentsFile =
        load_or_default_components(&config.output_dir.join("cache/components.yaml")).unwrap();
    assert!(
        top_level.components.iter().any(|c| !c.deleted),
        "fixture must produce at least one live component, got {top_level:#?}"
    );

    let mut per_component_count = 0usize;
    for entry in &top_level.components {
        if entry.deleted {
            continue;
        }
        let segment = entry
            .path_segments
            .first()
            .expect("every live component carries at least one path segment");
        let component_dir = tmp.path().join(&segment.path);
        let per_component_path = component_dir.join(".atlas").join("component.yaml");
        assert!(
            per_component_path.exists(),
            "expected per-component file at {} for component `{}`",
            per_component_path.display(),
            entry.id.as_str()
        );

        let bytes = std::fs::read(&per_component_path).unwrap();
        let parsed: PerComponentFile = serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "failed to parse {} as PerComponentFile: {e}",
                per_component_path.display()
            )
        });
        assert_eq!(parsed.schema_version, PER_COMPONENT_SCHEMA_VERSION);
        assert_eq!(parsed.surfaces_path.to_string_lossy(), "surfaces.yaml");
        assert_eq!(
            parsed.overrides_path.as_ref().unwrap().to_string_lossy(),
            "overrides.yaml"
        );
        assert!(
            !parsed.analyser_id.is_empty(),
            "analyser_id must be non-empty"
        );
        assert!(
            !parsed.analyser_version.is_empty(),
            "analyser_version must be non-empty"
        );
        assert!(
            !parsed.fingerprint.is_empty(),
            "fingerprint must be non-empty"
        );
        // The component projection round-trips against the top-level
        // record. Using equality of the embedded ComponentEntry —
        // the §4.6 invariant.
        assert_eq!(
            &parsed.component,
            entry,
            "per-component projection must equal the top-level slot for `{}`",
            entry.id.as_str()
        );
        per_component_count += 1;
    }

    assert!(
        per_component_count >= 2,
        "expected at least one per-component file per live component (≥2 for the fixture), got {per_component_count}"
    );
}

#[test]
fn cargo_classified_component_records_cargo_analyser_identity() {
    // PR-4 acceptance: a Cargo crate's
    // `<component>/.atlas/component.yaml` carries
    // `analyser_id: cargo-toml-classifier`, not the legacy
    // `l3-driver` placeholder. The dispatcher's per-analyser identity
    // plumbing must thread through to the projection.
    let tmp = TempDir::new().unwrap();
    write_lib(tmp.path(), "cargo-lib");

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds");
    assert!(summary.outputs_written);

    let per_component_path = tmp
        .path()
        .join("cargo-lib")
        .join(".atlas")
        .join("component.yaml");
    let bytes = std::fs::read(&per_component_path).expect("per-component file written");
    let parsed: PerComponentFile = serde_yaml::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed.analyser_id, "cargo-toml-classifier",
        "cargo crate must record cargo-toml-classifier as its analyser_id"
    );
    assert_eq!(
        parsed.analyser_version, "1.0.0",
        "cargo classifier version pins to 1.0.0 in the built-in registry"
    );
    assert_ne!(
        parsed.analyser_id, "l3-driver",
        "PR-4 deletes the l3-driver placeholder; per-component files must not still record it"
    );
}

#[test]
fn dockerfile_classified_component_records_dockerfile_analyser_identity() {
    // PR-4 acceptance: a Dockerfile-classified component records
    // `analyser_id: dockerfile-l3` — covering the second built-in
    // L3 deterministic analyser path.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("docker-svc");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Dockerfile"),
        "FROM rust:1.84-alpine\nWORKDIR /app\nCOPY . .\nCMD [\"/app/run\"]\n",
    )
    .unwrap();

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds");
    assert!(summary.outputs_written);

    let per_component_path = dir.join(".atlas").join("component.yaml");
    let bytes = std::fs::read(&per_component_path).expect("per-component file written");
    let parsed: PerComponentFile = serde_yaml::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed.analyser_id, "dockerfile-l3",
        "Dockerfile component must record dockerfile-l3 as its analyser_id, got `{}`",
        parsed.analyser_id
    );
    assert_eq!(parsed.analyser_version, "1.0.0");
}

/// PR-4 regression: an `overrides.additions` entry without a corresponding
/// pin must record `analyser_id: "override"` (not `"none"`) in its
/// per-component `component.yaml`. Previously `lookup_analyser_identity`
/// re-invoked `is_component`, which returned `"none"` for a directory that
/// no analyser recognised; the fix reads the identity from the L4 identity
/// map captured during assembly.
#[test]
fn overrides_addition_records_override_analyser_id() {
    let tmp = TempDir::new().unwrap();

    // Create a directory that no analyser would classify on its own
    // (no Cargo.toml, no Dockerfile, no package.json).
    let hand_dir = tmp.path().join("hand-authored-svc");
    std::fs::create_dir_all(&hand_dir).unwrap();
    std::fs::write(hand_dir.join("README.md"), "# hand-authored service\n").unwrap();

    // Write the overrides file to <root>/.atlas/components.overrides.yaml
    // (the location the CLI loads from).
    let atlas_dir = tmp.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).unwrap();
    std::fs::write(
        atlas_dir.join("components.overrides.yaml"),
        concat!(
            "schema_version: 1\n",
            "additions:\n",
            "  - id: hand-authored-svc\n",
            "    kind: service\n",
            "    evidence_grade: strong\n",
            "    rationale: hand-authored override\n",
            "    path_segments:\n",
            "      - path: hand-authored-svc\n",
            "        content_sha: \"\"\n",
        ),
    )
    .unwrap();

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds");
    assert!(summary.outputs_written);

    let per_component_path = hand_dir.join(".atlas").join("component.yaml");
    let bytes = std::fs::read(&per_component_path).unwrap_or_else(|e| {
        panic!(
            "per-component file must exist at {}: {e}",
            per_component_path.display()
        )
    });
    let parsed: PerComponentFile = serde_yaml::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", per_component_path.display()));
    assert_eq!(
        parsed.analyser_id, "override",
        "overrides.additions entry must record analyser_id: \"override\", got `{}`",
        parsed.analyser_id
    );
    assert_eq!(
        parsed.analyser_version, "0.0.0",
        "overrides.additions entry must record analyser_version: \"0.0.0\", got `{}`",
        parsed.analyser_version
    );
}

#[test]
fn dry_run_skips_per_component_writes() {
    let tmp = TempDir::new().unwrap();
    write_lib(tmp.path(), "dry-lib");

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config.dry_run = true;
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("dry-run run_index succeeds");
    assert!(!summary.outputs_written);

    // The fixture's only component is `dry-lib`. No
    // `<root>/dry-lib/.atlas/component.yaml` may exist.
    let per_component = tmp.path().join("dry-lib/.atlas/component.yaml");
    assert!(
        !per_component.exists(),
        "dry-run must not write per-component files; found {}",
        per_component.display()
    );
}
