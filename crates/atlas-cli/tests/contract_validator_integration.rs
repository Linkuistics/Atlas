//! Integration test for PR-8 acceptance criterion #3:
//!
//! A `related-components.yaml` that contains a `consumes-contract` edge
//! whose contract id resolves to no defining component must cause `run_index`
//! to return `Err(IndexError::Other(...))` — i.e. "fails the run".
//!
//! This test proves that `validate_contract_participants_resolve` is actually
//! wired into the pipeline (not merely tested in unit isolation).

use std::sync::{Arc, Mutex};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig, IndexError};
use atlas_llm::{LlmBackend, LlmFingerprint, LlmRequest, PromptId};
use serde_json::{json, Value};

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [9u8; 32],
        ontology_sha: [9u8; 32],
        model_id: "contract-validator-test-backend".into(),
        backend_version: "v-test".into(),
    }
}

/// Backend that behaves like `LenientBackend` for all prompts except
/// `Stage2Edges`, where it always returns a single `consumes-contract` edge
/// whose contract participant (`participants[1]`) does **not** exist in any
/// component's `surfaces.yaml`.  This forces the PR-8 validator inside
/// `run_index` to detect an unresolved participant and return an error.
struct ContractViolationBackend {
    fingerprint: LlmFingerprint,
    /// Tracks how many times Stage2Edges was called (for sanity assertions).
    stage2_calls: Mutex<u32>,
}

impl ContractViolationBackend {
    fn new() -> Arc<Self> {
        Arc::new(ContractViolationBackend {
            fingerprint: fingerprint(),
            stage2_calls: Mutex::new(0),
        })
    }
}

impl LlmBackend for ContractViolationBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, atlas_llm::LlmError> {
        match req.prompt_template {
            PromptId::Classify => Ok(json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "classifier default",
                "is_boundary": true,
            })),
            PromptId::Stage1Surface => Ok(json!({
                "purpose": "contract validator test component",
                "notes": "",
            })),
            PromptId::Stage2Edges => {
                *self.stage2_calls.lock().unwrap() += 1;
                // "mylib" is a real component id produced by the tiny fixture.
                // "nonexistent/fake-contract" is deliberately absent from
                // every component's surfaces.yaml — the validator must reject it.
                Ok(json!([{
                    "kind": "consumes-contract",
                    "lifecycle": "design",
                    "participants": ["mylib", "nonexistent/fake-contract"],
                    "evidence_grade": "strong",
                    "evidence_fields": ["mylib.consumes"],
                    "rationale": "synthetic edge for validator integration test",
                }]))
            }
            PromptId::Subcarve => Ok(json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "policy declined",
            })),
        }
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

// ---------------------------------------------------------------------------
// Fixture helpers — mirrors the pattern in pipeline_integration.rs
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn tiny_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tiny")
}

fn copy_fixture_to_tmp(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_fixture_to_tmp(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

fn materialise_tiny_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    copy_fixture_to_tmp(&tiny_fixture_root(), tmp.path());
    tmp
}

fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

// ---------------------------------------------------------------------------
// The integration test
// ---------------------------------------------------------------------------

/// Acceptance criterion #3 (PR-8): a `consumes-contract` edge whose contract
/// participant resolves to no defining component must cause `run_index` to
/// return `Err(IndexError::Other(...))` mentioning either `"unresolved"` or
/// the unresolved id `"nonexistent/fake-contract"`.
#[test]
fn unresolved_contract_participant_fails_run_index() {
    let tmp = materialise_tiny_fixture();
    let backend = ContractViolationBackend::new();
    let config = base_config(tmp.path());

    let result = run_index(
        &config,
        backend.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    );

    let err = result.unwrap_err();
    assert!(
        matches!(err, IndexError::Other(_)),
        "expected IndexError::Other for unresolved contract participant, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("unresolved") || msg.contains("nonexistent/fake-contract"),
        "error message must mention 'unresolved' or the unresolved id; got: {msg}"
    );

    // Sanity: Stage2Edges was actually called, so the canned response
    // entered the pipeline rather than being skipped by a cache hit.
    assert!(
        *backend.stage2_calls.lock().unwrap() > 0,
        "Stage2Edges must have been called at least once for the canned response to enter the pipeline"
    );
}
