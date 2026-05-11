//! Phase 6 PR-4: dual-mode contract test for `--strict-overrides`.
//!
//! Exercises every variant of the closed `OverrideWarning` enumeration
//! in both modes:
//!   - Permissive (no `--strict-overrides`): warning text on stderr;
//!     exit 0.
//!   - Strict (`--strict-overrides`): warning text on stderr; exit
//!     non-zero.
//!
//! This subsumes the deferred Phase 3 PR-10 stderr-capture test for
//! `edges_suppress no-match` (now one of three variants).
//!
//! ## Test harness shape
//!
//! The verbatim test reference in the PR-4 plan invokes the binary
//! as a subprocess. The CLI's production backend stack requires a
//! filesystem-access LLM provider (`claude-code` / `codex`) on the
//! host's PATH, so a subprocess-only contract test would be
//! `claude`-gated like `agent_observer_e2e.rs`. To exercise the
//! contract under `cargo test --workspace --release` on every CI
//! lane, we instead drive `run_index` library-side with a
//! `LenientBackend` stub and a [`CapturingCollector`] that records
//! warning emits in-memory. The exit-code contract is mapped to the
//! `Result` returned by `run_index`:
//!
//! - Permissive: `IndexError::StrictOverridesFailed` is NEVER returned
//!   even when warnings fire — the run completes successfully.
//! - Strict: `IndexError::StrictOverridesFailed` is returned exactly
//!   when at least one closed-enumeration warning fired during the run.
//!
//! The `RunOutput` shim mirrors `std::process::Output` so the asserts
//! in the contract are written in the natural `stderr`/`status.code()`
//! style.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig, IndexError};
use atlas_engine::testing::LenientBackend;
use atlas_engine::{CapturingCollector, OverrideWarningCollector};
use atlas_llm::LlmFingerprint;
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0x77u8; 32],
        ontology_sha: [0x78u8; 32],
        model_id: "phase6-pr4-backend".into(),
        backend_version: "v-phase6-pr4".into(),
    }
}

/// Captures the result of a `run_index` invocation in a shape mirroring
/// `std::process::Output`. `status` is `Ok` on success;
/// `stderr` carries the captured warning text from the
/// `CapturingCollector`; `exit_code` mirrors the CLI's mapping from
/// `IndexError` to process exit code.
struct RunOutput {
    status: Result<atlas_cli::IndexSummary, IndexError>,
    stderr: String,
    exit_code: i32,
}

impl RunOutput {
    fn stderr(&self) -> &str {
        &self.stderr
    }
    fn status_code(&self) -> i32 {
        self.exit_code
    }
}

/// Drive `run_index` against `root` with a permissive collector
/// (default mode). Returns the captured warning text plus the exit
/// code the CLI would emit.
fn run_atlas_index(root: &Path) -> RunOutput {
    run_atlas_index_inner(root, /* strict_overrides */ false)
}

/// Drive `run_index` against `root` with `--strict-overrides`
/// semantics — install a strict capturing collector and assert the
/// `StrictOverridesFailed` exit-code path is exercised when warnings
/// fire.
fn run_atlas_index_with_args(root: &Path, args: &[&str]) -> RunOutput {
    let strict = args.contains(&"--strict-overrides");
    run_atlas_index_inner(root, strict)
}

fn run_atlas_index_inner(root: &Path, strict_overrides: bool) -> RunOutput {
    // Build a capturing collector that mirrors the production
    // permissive/strict policy so the test exercises the same
    // `has_errors()` branch the CLI consults.
    let collector: Arc<CapturingCollector> = Arc::new(if strict_overrides {
        CapturingCollector::new_strict()
    } else {
        CapturingCollector::new_permissive()
    });

    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    // The capturing collector mirrors strict vs permissive on its own;
    // do NOT also set `strict_overrides: true` (that would install a
    // second `StrictCollector` that competes with ours). The
    // `override_warning_collector` override-path wins.
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
    // Map the library-side `IndexError` taxonomy to the CLI's exit-code
    // mapping at `crates/atlas-cli/src/main.rs:run_index_cmd`:
    //   Ok                            -> 0
    //   StrictOverridesFailed         -> 4
    //   BudgetExhausted               -> 2
    //   SetupFailed                   -> 3
    //   Other                         -> 1
    let exit_code = match &status {
        Ok(_) => 0,
        Err(IndexError::StrictOverridesFailed(_)) => 4,
        Err(IndexError::BudgetExhausted) => 2,
        Err(IndexError::SetupFailed(_)) => 3,
        Err(IndexError::Other(_)) => 1,
    };
    RunOutput {
        status,
        stderr,
        exit_code,
    }
}

/// Materialise a minimal Cargo library crate at `<root>/<rel_path>`
/// so L4 produces a live component.
fn write_cargo_lib(root: &Path, rel_path: &str) {
    let dir = root.join(rel_path);
    let name = dir.file_name().unwrap().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
}

// =============================================================
// Fixtures — each one triggers exactly one variant of the
// closed `OverrideWarning` enumeration.
// =============================================================

fn fixture_with_edges_suppress_no_match(tmp: &TempDir) -> PathBuf {
    let root = tmp.path().to_path_buf();
    write_cargo_lib(&root, "crate-a");
    write_cargo_lib(&root, "crate-b");
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(
        root.join(".atlas/components.overrides.yaml"),
        "schema_version: 1\n\
         edges_suppress:\n  \
         - kind: depends-on\n    \
           from: crate-a\n    \
           to: nonexistent-crate\n    \
           reason: \"no analyser edge to suppress\"\n",
    )
    .unwrap();
    root
}

fn fixture_with_edges_add_unknown_kind(tmp: &TempDir) -> PathBuf {
    let root = tmp.path().to_path_buf();
    write_cargo_lib(&root, "crate-a");
    write_cargo_lib(&root, "crate-b");
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(
        root.join(".atlas/components.overrides.yaml"),
        "schema_version: 1\n\
         edges_add:\n  \
         - kind: bogus-not-a-real-kind\n    \
           from: crate-a\n    \
           to: crate-b\n    \
           reason: \"forces EdgesAddUnknownKind\"\n",
    )
    .unwrap();
    root
}

fn fixture_with_subsystem_override_nonexistent(tmp: &TempDir) -> PathBuf {
    let root = tmp.path().to_path_buf();
    write_cargo_lib(&root, "crate-a");
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(
        root.join(".atlas/subsystems.overrides.yaml"),
        "schema_version: 1\n\
         subsystems:\n  \
         - id: gamma\n    \
           members:\n      \
           - nonexistent-component\n    \
           rationale: \"forces SubsystemOverrideNonExistent\"\n    \
           evidence_grade: strong\n",
    )
    .unwrap();
    root
}

// =============================================================
// Variant 1: EdgesSuppressNoMatch
// =============================================================

#[test]
fn edges_suppress_no_match_permissive_emits_warning_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let root = fixture_with_edges_suppress_no_match(&tmp);
    let output = run_atlas_index(&root);
    let stderr = output.stderr();
    assert!(
        stderr.contains("edges_suppress")
            || stderr.contains("no match")
            || stderr.contains("matched no edges"),
        "expected edges_suppress no-match warning text on stderr; got: {stderr}"
    );
    assert_eq!(
        output.status_code(),
        0,
        "permissive mode must exit 0 even when the warning fires; got status={:?}",
        output.status.as_ref().err()
    );
}

#[test]
fn edges_suppress_no_match_strict_emits_warning_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    let root = fixture_with_edges_suppress_no_match(&tmp);
    let output = run_atlas_index_with_args(&root, &["--strict-overrides"]);
    let stderr = output.stderr();
    assert!(
        stderr.contains("edges_suppress")
            || stderr.contains("no match")
            || stderr.contains("matched no edges"),
        "expected edges_suppress no-match warning text on stderr; got: {stderr}"
    );
    assert_ne!(
        output.status_code(),
        0,
        "strict mode must exit non-zero when the warning fires"
    );
    assert!(matches!(
        output.status,
        Err(IndexError::StrictOverridesFailed(_))
    ));
}

// =============================================================
// Variant 2: EdgesAddUnknownKind
// =============================================================

#[test]
fn edges_add_unknown_kind_permissive_emits_warning_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let root = fixture_with_edges_add_unknown_kind(&tmp);
    let output = run_atlas_index(&root);
    let stderr = output.stderr();
    assert!(
        stderr.contains("bogus-not-a-real-kind") || stderr.contains("unknown kind"),
        "expected edges_add unknown-kind warning text on stderr; got: {stderr}"
    );
    assert_eq!(
        output.status_code(),
        0,
        "permissive mode must exit 0; got status={:?}",
        output.status.as_ref().err()
    );
}

#[test]
fn edges_add_unknown_kind_strict_emits_warning_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    let root = fixture_with_edges_add_unknown_kind(&tmp);
    let output = run_atlas_index_with_args(&root, &["--strict-overrides"]);
    let stderr = output.stderr();
    assert!(
        stderr.contains("bogus-not-a-real-kind") || stderr.contains("unknown kind"),
        "expected edges_add unknown-kind warning text on stderr; got: {stderr}"
    );
    assert_ne!(output.status_code(), 0, "strict mode must exit non-zero");
    assert!(matches!(
        output.status,
        Err(IndexError::StrictOverridesFailed(_))
    ));
}

// =============================================================
// Variant 3: SubsystemOverrideNonExistent
// =============================================================

#[test]
fn subsystem_override_nonexistent_permissive_emits_warning_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let root = fixture_with_subsystem_override_nonexistent(&tmp);
    let output = run_atlas_index(&root);
    let stderr = output.stderr();
    assert!(
        stderr.contains("nonexistent-component"),
        "expected subsystem-override warning to name the missing component; got: {stderr}"
    );
    assert_eq!(
        output.status_code(),
        0,
        "permissive mode must exit 0; got status={:?}",
        output.status.as_ref().err()
    );
}

#[test]
fn subsystem_override_nonexistent_strict_emits_warning_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    let root = fixture_with_subsystem_override_nonexistent(&tmp);
    let output = run_atlas_index_with_args(&root, &["--strict-overrides"]);
    let stderr = output.stderr();
    assert!(
        stderr.contains("nonexistent-component"),
        "expected subsystem-override warning to name the missing component; got: {stderr}"
    );
    assert_ne!(output.status_code(), 0, "strict mode must exit non-zero");
    assert!(matches!(
        output.status,
        Err(IndexError::StrictOverridesFailed(_))
    ));
}
