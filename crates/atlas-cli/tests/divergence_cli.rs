//! Phase 3 PR-11 CLI integration tests for `atlas divergence`.
//!
//! Each test follows the same pattern: materialise a tiny fixture
//! under a TempDir, run `atlas_cli::run_index` once with a permissive
//! canned-response backend so the engine cache and the four Atlas
//! YAMLs land on disk, then drive [`atlas_cli::reports::run_divergence`]
//! against the same fixture and assert the disk state.
//!
//! The tests deliberately hand-craft the drift snapshot file when one
//! is needed — PR-11 lands ahead of (or in parallel with) PR-8, so we
//! cannot depend on `atlas drift` to produce the snapshot. Hand-
//! crafting matches the wire format `atlas-reports::ContractShaSnapshot`
//! consumes and locks the divergence handler's read path independently
//! of the writer's eventual landing order.

use std::path::{Path, PathBuf};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::reports::{run_divergence, DivergenceOptions, OutputFormat};
use atlas_cli::{run_index, IndexConfig};
use atlas_engine::testing::LenientBackend;
use atlas_llm::LlmFingerprint;
use atlas_reports::{ContractShaEntry, ContractShaSnapshot, DivergenceReport};
use chrono::{TimeZone, Utc};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [11u8; 32],
        ontology_sha: [12u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v-divergence-test".into(),
    }
}

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

/// Run `atlas index` once with a fresh backend so the engine cache
/// and the four Atlas YAMLs land on disk under `<root>/.atlas/`.
fn run_index_once(root: &Path) {
    let config = base_config(root);
    let backend = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index must succeed");
}

/// Build the [`DivergenceOptions`] for the given root, mirroring the
/// `IndexConfig` setup that `run_index_once` used.
fn divergence_opts(root: &Path) -> DivergenceOptions {
    DivergenceOptions {
        root: root.to_path_buf(),
        output_dir: root.join(".atlas"),
        no_write: false,
        format: OutputFormat::Yaml,
        fingerprint_override: Some(fingerprint()),
    }
}

/// Hand-craft a drift snapshot under the workspace's `.atlas/cache/`.
/// Returns the absolute path to the snapshot file.
fn write_drift_snapshot(root: &Path, entries: Vec<ContractShaEntry>) -> PathBuf {
    let snapshot = ContractShaSnapshot {
        schema_version: 1,
        captured_at: Utc.with_ymd_and_hms(2026, 5, 7, 9, 11, 42).unwrap(),
        contract_shas: entries,
    };
    let path = root.join(".atlas/cache/contract-shas-snapshot.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let yaml = serde_yaml::to_string(&snapshot).unwrap();
    std::fs::write(&path, yaml).unwrap();
    path
}

fn report_path(root: &Path) -> PathBuf {
    root.join(".atlas/cache/reports/composition-divergence.yaml")
}

fn read_report(root: &Path) -> DivergenceReport {
    let bytes = std::fs::read(report_path(root)).expect("composition-divergence.yaml exists");
    serde_yaml::from_slice(&bytes).expect("composition-divergence.yaml parses")
}

// --------------------------------------------------------------------
// Acceptance criteria
// --------------------------------------------------------------------

/// AC: divergence run after a hand-crafted drift snapshot has severity
/// values populated (i.e. not `null`) and the report header carries
/// the snapshot's `captured_at`.
///
/// The tiny fixture is an under-coupled workspace (two unrelated
/// rust crates) so we are testing the wire-up rather than a particular
/// pair classification: the assertion is that whichever pairs the
/// engine produces, every divergent pair carries a numeric severity.
#[test]
fn atlas_divergence_after_drift_writes_severity_aware_report() {
    let tmp = materialise_tiny_fixture();
    run_index_once(tmp.path());

    // Hand-craft a baseline. The exact contract ids do not matter for
    // this AC — we just need a snapshot file present so the report's
    // baseline-aware code path fires.
    write_drift_snapshot(
        tmp.path(),
        vec![ContractShaEntry {
            id: "atlas-contracts/placeholder/v1".into(),
            content_sha: "sha256:baseline-placeholder".into(),
        }],
    );

    let opts = divergence_opts(tmp.path());
    let backend = LenientBackend::new(fingerprint());
    let report = run_divergence(&opts, backend).expect("run_divergence must succeed");

    assert_eq!(report.schema_version, 1);
    assert!(
        report.drift_baseline_at.is_some(),
        "header must carry the baseline `captured_at`"
    );
    for pair in &report.divergent_pairs {
        assert!(
            pair.severity.is_some(),
            "divergent pair must carry a numeric severity when a baseline is present: {pair:?}"
        );
    }
    assert!(
        report_path(tmp.path()).exists(),
        "report file must be on disk"
    );

    // The on-disk report must round-trip back to the in-memory shape.
    let on_disk = read_report(tmp.path());
    assert_eq!(on_disk, report);
}

/// AC: divergence run with no prior drift baseline → header notes
/// baseline absent (`drift_baseline_at: None`), severity is `None` for
/// every pair.
#[test]
fn atlas_divergence_without_prior_drift_writes_null_severity_report() {
    let tmp = materialise_tiny_fixture();
    run_index_once(tmp.path());

    // Ensure no snapshot exists.
    let snapshot_path = tmp.path().join(".atlas/cache/contract-shas-snapshot.yaml");
    assert!(!snapshot_path.exists());

    let opts = divergence_opts(tmp.path());
    let backend = LenientBackend::new(fingerprint());
    let report = run_divergence(&opts, backend).expect("run_divergence must succeed");

    assert_eq!(
        report.drift_baseline_at, None,
        "drift_baseline_at must be None when no snapshot is on disk"
    );
    for pair in &report.divergent_pairs {
        assert_eq!(
            pair.severity, None,
            "every divergent pair has null severity when no baseline exists: {pair:?}"
        );
        assert!(pair.drifting_contracts.is_empty());
    }
    assert!(
        report.summary.by_severity.is_empty(),
        "by_severity histogram is empty in the no-baseline case"
    );
}

/// AC: `--no-write` honoured — the report is computed and rendered
/// to stdout, but no file lands under
/// `<output>/.atlas/cache/reports/`.
#[test]
fn atlas_divergence_no_write_skips_writes() {
    let tmp = materialise_tiny_fixture();
    run_index_once(tmp.path());

    let mut opts = divergence_opts(tmp.path());
    opts.no_write = true;

    let backend = LenientBackend::new(fingerprint());
    let report = run_divergence(&opts, backend).expect("run_divergence must succeed");

    // The report itself is still produced.
    assert_eq!(report.schema_version, 1);
    // But nothing landed under reports/.
    assert!(
        !report_path(tmp.path()).exists(),
        "composition-divergence.yaml must NOT be written when --no-write is set"
    );
}

/// AC: divergence run does not modify the drift snapshot. The handler
/// is read-only on `contract-shas-snapshot.yaml`; this test pre-
/// populates a baseline, runs divergence, and asserts the file's
/// bytes are unchanged.
///
/// PR-8 is the writer of the snapshot; PR-11 is intentionally read-
/// only on it. This is the regression guard the plan §4 PR-11 ACs
/// call out.
#[test]
fn atlas_divergence_does_not_modify_drift_snapshot() {
    let tmp = materialise_tiny_fixture();
    run_index_once(tmp.path());

    let snapshot_path = write_drift_snapshot(
        tmp.path(),
        vec![
            ContractShaEntry {
                id: "atlas-contracts/eval-schema/v1".into(),
                content_sha: "sha256:def456".into(),
            },
            ContractShaEntry {
                id: "atlas-contracts/index-schema/v1".into(),
                content_sha: "sha256:abc123".into(),
            },
        ],
    );
    let before_bytes = std::fs::read(&snapshot_path).expect("snapshot readable before run");
    let before_mtime = std::fs::metadata(&snapshot_path)
        .expect("snapshot metadata readable before run")
        .modified()
        .ok();

    let opts = divergence_opts(tmp.path());
    let backend = LenientBackend::new(fingerprint());
    run_divergence(&opts, backend).expect("run_divergence must succeed");

    let after_bytes = std::fs::read(&snapshot_path).expect("snapshot readable after run");
    let after_mtime = std::fs::metadata(&snapshot_path)
        .expect("snapshot metadata readable after run")
        .modified()
        .ok();

    assert_eq!(
        before_bytes, after_bytes,
        "divergence must not rewrite the drift snapshot"
    );
    if let (Some(before), Some(after)) = (before_mtime, after_mtime) {
        assert_eq!(
            before, after,
            "snapshot mtime must be unchanged after divergence"
        );
    }
}
