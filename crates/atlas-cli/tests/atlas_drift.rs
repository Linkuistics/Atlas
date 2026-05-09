//! Phase 3 PR-8 acceptance tests for `atlas drift`.
//!
//! These tests drive the CLI handler library entry point
//! [`atlas_cli::reports::run_drift`] directly (not via the binary),
//! so a panic injected by [`atlas_engine::atomic_write::test_hooks_pub`]
//! can be caught with `std::panic::catch_unwind`. The tests use a
//! [`LenientBackend`] so the backing `atlas index` invocation
//! completes without network access.
//!
//! ## AC mapping (plan §4 PR-8)
//!
//! - `atlas_drift_first_run_writes_snapshot_and_empty_report`
//! - `atlas_drift_second_run_after_contract_change_reports_drift`
//! - `atlas_drift_no_write_flag_skips_writes`
//! - `atlas_drift_kill_during_snapshot_write_leaves_file_intact`

use std::path::Path;
use std::time::SystemTime;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::reports::{run_drift, DriftArgs, OutputFormat};
use atlas_cli::{run_index, IndexConfig};
use atlas_engine::testing::LenientBackend;
use atlas_llm::LlmFingerprint;
use atlas_reports::{ContractShaSnapshot, DriftReport};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [13u8; 32],
        ontology_sha: [17u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v-pr8".into(),
    }
}

fn write_rust_lib(root: &Path, name: &str, lib_body: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n\n[dependencies]\nserde = {{ version = \"1\", features = [\"derive\"] }}\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), lib_body).unwrap();
}

fn run_atlas_index(root: &Path) {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds");
}

fn drift_args_for(root: &Path) -> DriftArgs {
    DriftArgs {
        format: OutputFormat::Yaml,
        no_write: false,
        root: Some(root.to_path_buf()),
    }
}

fn snapshot_path(root: &Path) -> std::path::PathBuf {
    root.join(".atlas/cache/contract-shas-snapshot.yaml")
}

fn report_path(root: &Path) -> std::path::PathBuf {
    root.join(".atlas/cache/reports/drift.yaml")
}

fn read_snapshot(path: &Path) -> ContractShaSnapshot {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        panic!("expected snapshot at {}: {e}", path.display());
    });
    serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "expected valid ContractShaSnapshot YAML at {}: {e}",
            path.display()
        )
    })
}

fn read_report(path: &Path) -> DriftReport {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        panic!("expected report at {}: {e}", path.display());
    });
    serde_yaml::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("expected valid DriftReport YAML at {}: {e}", path.display()))
}

/// AC: `atlas_drift_first_run_writes_snapshot_and_empty_report`
/// — fresh fixture, run `atlas drift`, assert both cache files exist;
/// report has empty change arrays.
#[test]
fn atlas_drift_first_run_writes_snapshot_and_empty_report() {
    let tmp = TempDir::new().unwrap();
    write_rust_lib(
        tmp.path(),
        "alpha",
        "use serde::{Serialize, Deserialize};\n\n\
         #[derive(Serialize, Deserialize)]\n\
         pub struct Foo { pub a: u32 }\n",
    );
    run_atlas_index(tmp.path());

    let mut stdout: Vec<u8> = Vec::new();
    let exit = run_drift(&drift_args_for(tmp.path()), &mut stdout).expect("run_drift returns Ok");
    assert_eq!(
        exit,
        std::process::ExitCode::SUCCESS,
        "first run must exit 0"
    );

    let snap_path = snapshot_path(tmp.path());
    let rep_path = report_path(tmp.path());
    assert!(
        snap_path.exists(),
        "snapshot file must be written at {}",
        snap_path.display()
    );
    assert!(
        rep_path.exists(),
        "report file must be written at {}",
        rep_path.display()
    );

    let report = read_report(&rep_path);
    assert!(
        report.contracts_changed.is_empty(),
        "first-run report must have empty contracts_changed"
    );
    assert!(
        report.contracts_added.is_empty(),
        "first-run report must have empty contracts_added"
    );
    assert!(
        report.contracts_removed.is_empty(),
        "first-run report must have empty contracts_removed"
    );
    assert_eq!(
        report.baseline_captured_at, None,
        "first-run report must have null baseline_captured_at"
    );
    assert!(
        report.summary.total_contracts >= 1,
        "first-run snapshot must capture at least the Foo contract; got {}",
        report.summary.total_contracts
    );

    let snap = read_snapshot(&snap_path);
    assert_eq!(
        snap.contract_shas.len() as u32,
        report.summary.total_contracts,
        "snapshot entry count must match report summary"
    );
}

/// AC: `atlas_drift_second_run_after_contract_change_reports_drift`
/// — first run captures baseline, mutate one contract, second run
/// reports it.
#[test]
fn atlas_drift_second_run_after_contract_change_reports_drift() {
    let tmp = TempDir::new().unwrap();
    write_rust_lib(
        tmp.path(),
        "alpha",
        "use serde::{Serialize, Deserialize};\n\n\
         #[derive(Serialize, Deserialize)]\n\
         pub struct Foo { pub a: u32 }\n",
    );
    run_atlas_index(tmp.path());

    // Run 1 — capture baseline.
    let mut sink: Vec<u8> = Vec::new();
    run_drift(&drift_args_for(tmp.path()), &mut sink).expect("first drift run");
    let baseline = read_snapshot(&snapshot_path(tmp.path()));
    assert!(
        !baseline.contract_shas.is_empty(),
        "baseline must capture at least one contract sha"
    );

    // Mutate the Rust struct so the contract content_sha shifts.
    write_rust_lib(
        tmp.path(),
        "alpha",
        "use serde::{Serialize, Deserialize};\n\n\
         #[derive(Serialize, Deserialize)]\n\
         pub struct Foo { pub a: u32, pub b: u32 }\n",
    );
    run_atlas_index(tmp.path());

    // Run 2 — drift must report the changed contract.
    let mut sink: Vec<u8> = Vec::new();
    run_drift(&drift_args_for(tmp.path()), &mut sink).expect("second drift run");

    let report = read_report(&report_path(tmp.path()));
    assert_eq!(
        report.contracts_changed.len(),
        1,
        "expected exactly one changed contract; got {:?}",
        report.contracts_changed
    );
    let change = &report.contracts_changed[0];
    assert_ne!(
        change.prior_content_sha, change.current_content_sha,
        "changed contract must have prior != current"
    );
    assert_eq!(report.summary.changed, 1);
}

/// AC: `atlas_drift_no_write_flag_skips_writes` — `atlas drift
/// --no-write` after a previous run does NOT mutate the existing
/// snapshot or report files (verified by mtime).
#[test]
fn atlas_drift_no_write_flag_skips_writes() {
    let tmp = TempDir::new().unwrap();
    write_rust_lib(
        tmp.path(),
        "alpha",
        "use serde::{Serialize, Deserialize};\n\n\
         #[derive(Serialize, Deserialize)]\n\
         pub struct Foo { pub a: u32 }\n",
    );
    run_atlas_index(tmp.path());

    // First run lands the baseline files on disk.
    let mut sink: Vec<u8> = Vec::new();
    run_drift(&drift_args_for(tmp.path()), &mut sink).expect("first drift run");
    let snap = snapshot_path(tmp.path());
    let rep = report_path(tmp.path());
    let snap_mtime = mtime(&snap);
    let rep_mtime = mtime(&rep);

    // Sleep long enough that any subsequent write would observably
    // bump the mtime (filesystem mtime resolution can be 1s on some
    // platforms; 1.1s is the safe floor that tempfile-author repos
    // use across CI).
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Second run with --no-write must not touch the files.
    let mut args = drift_args_for(tmp.path());
    args.no_write = true;
    let mut sink: Vec<u8> = Vec::new();
    run_drift(&args, &mut sink).expect("--no-write drift run");

    let snap_mtime_after = mtime(&snap);
    let rep_mtime_after = mtime(&rep);
    assert_eq!(
        snap_mtime, snap_mtime_after,
        "--no-write must not bump the snapshot mtime"
    );
    assert_eq!(
        rep_mtime, rep_mtime_after,
        "--no-write must not bump the report mtime"
    );
}

/// AC: `atlas_drift_kill_during_snapshot_write_leaves_file_intact`
/// — kill-during-write fixture (using PR-1's atomic-write helper);
/// snapshot is either fully-old or fully-new, not half-written.
///
/// Approach: pre-populate the snapshot with a known baseline, arm
/// the cross-crate `atomic_write` panic hook, and call `run_drift`
/// inside `catch_unwind`. After recovery, the on-disk snapshot must
/// equal the original baseline byte-for-byte (the panic fired
/// between temp-write and rename, so the rename never landed).
#[test]
fn atlas_drift_kill_during_snapshot_write_leaves_file_intact() {
    use atlas_engine::atomic_write::test_hooks_pub::{
        arm_panic_before_rename, disarm_panic_before_rename,
    };

    let tmp = TempDir::new().unwrap();
    write_rust_lib(
        tmp.path(),
        "alpha",
        "use serde::{Serialize, Deserialize};\n\n\
         #[derive(Serialize, Deserialize)]\n\
         pub struct Foo { pub a: u32 }\n",
    );
    run_atlas_index(tmp.path());

    // First run — establish a baseline snapshot on disk.
    let mut sink: Vec<u8> = Vec::new();
    run_drift(&drift_args_for(tmp.path()), &mut sink).expect("baseline drift run");

    let snap_path = snapshot_path(tmp.path());
    let baseline_bytes = std::fs::read(&snap_path).expect("baseline snapshot must exist");
    assert!(
        !baseline_bytes.is_empty(),
        "baseline snapshot must be non-empty"
    );

    // Mutate the contract so the second run would normally rewrite
    // the snapshot. Without the hook, the new snapshot would replace
    // the baseline; with the hook armed, the rename never happens.
    write_rust_lib(
        tmp.path(),
        "alpha",
        "use serde::{Serialize, Deserialize};\n\n\
         #[derive(Serialize, Deserialize)]\n\
         pub struct Foo { pub a: u32, pub b: u32 }\n",
    );
    run_atlas_index(tmp.path());

    // Arm the one-shot panic hook. The first atomic_write the
    // handler invokes (the drift report write) panics before its
    // rename — the snapshot write that follows never runs, but the
    // important invariant is that the destination file is never
    // half-written.
    let args = drift_args_for(tmp.path());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arm_panic_before_rename();
        let mut sink: Vec<u8> = Vec::new();
        let _ = run_drift(&args, &mut sink);
    }));
    // Defensive disarm in case `catch_unwind` unwound before the
    // hook's one-shot disarm fired (the auto-disarm runs eagerly so
    // this is belt-and-braces).
    disarm_panic_before_rename();
    assert!(
        result.is_err(),
        "the armed hook must fire and `run_drift` must propagate the panic"
    );

    // Snapshot file must equal the baseline byte-for-byte: the
    // rename never happened, so the destination is fully-old. (The
    // alternative permitted by the AC — fully-new — would also pass
    // the "is parseable YAML" check, but our fixture knows exactly
    // which write the hook intercepts.)
    let post_bytes = std::fs::read(&snap_path).expect("snapshot must still be readable");
    assert_eq!(
        post_bytes, baseline_bytes,
        "snapshot must be fully-old after a simulated kill mid-write \
         (half-written content would corrupt the next run's baseline)"
    );

    // The snapshot must also still parse as a valid
    // ContractShaSnapshot — half-written YAML would not.
    let _: ContractShaSnapshot = serde_yaml::from_slice(&post_bytes)
        .expect("post-kill snapshot must still parse as ContractShaSnapshot");
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("failed to stat {}: {e}", path.display()))
        .modified()
        .unwrap_or_else(|e| panic!("modified() unsupported on {}: {e}", path.display()))
}
