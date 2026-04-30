//! Regression test: decomposing L5 demand into a per-component loop
//! must produce byte-identical `components.yaml` output. Spec §7.3.

use std::path::Path;
use std::sync::Arc;

use atlas_cli::pipeline::{run_index, IndexConfig};
use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_index::ComponentsFile;
use atlas_llm::{LlmBackend, LlmFingerprint, PromptId, TestBackend};
use serde_json::json;
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [9u8; 32],
        ontology_sha: [1u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
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
    std::fs::write(dir.join("src").join("lib.rs"), "// lib\n").unwrap();
}

fn make_backend() -> Arc<dyn LlmBackend> {
    let backend = TestBackend::with_fingerprint(fingerprint());
    backend.respond(PromptId::Stage1Surface, json!({}), json!({"purpose": "x"}));
    backend.respond(PromptId::Stage2Edges, json!({}), json!([]));
    Arc::new(backend) as Arc<dyn LlmBackend>
}

#[test]
fn pipeline_run_twice_produces_identical_components_yaml() {
    let tmp = TempDir::new().unwrap();
    write_lib(tmp.path(), "lib");

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());

    let reporter = make_stderr_reporter(ProgressMode::Never, None);
    let backend = make_backend();
    let _ = run_index(&config, backend, None, Arc::clone(&reporter)).unwrap();
    let first = std::fs::read(config.output_dir.join("components.yaml")).unwrap();

    let reporter2 = make_stderr_reporter(ProgressMode::Never, None);
    let backend2 = make_backend();
    let _ = run_index(&config, backend2, None, Arc::clone(&reporter2)).unwrap();
    let second = std::fs::read(config.output_dir.join("components.yaml")).unwrap();

    assert_eq!(
        first, second,
        "components.yaml must be byte-identical on no-op re-run"
    );

    let parsed_first: ComponentsFile = serde_yaml::from_slice(&first).unwrap();
    let parsed_second: ComponentsFile = serde_yaml::from_slice(&second).unwrap();
    assert_eq!(parsed_first, parsed_second);
}
