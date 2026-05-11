//! Phase 6 PR-4: dual-mode contract test for `--strict-overrides`.
//!
//! Exercises every variant of the closed `OverrideWarning` enumeration
//! in both modes:
//!   - Permissive (no `--strict-overrides`): warning text on stderr;
//!     exit 0.
//!   - Strict (`--strict-overrides`): warning text on stderr; exit
//!     non-zero (`4`).
//!
//! This subsumes the deferred Phase 3 PR-10 stderr-capture test for
//! `edges_suppress no-match` (now one of three variants).
//!
//! ## Test harness shape
//!
//! Subprocess invocation of the `atlas` binary requires `claude-code`
//! (or `codex`) on PATH because L3's `is_component` classify call hits
//! the LLM before L6's override warnings fire on the merged YAML.
//! Gating the contract test on `ATLAS_LLM_RUN_CLAUDE_TESTS=1` would
//! skip it on every default-CI lane — defeating the whole point of a
//! `--strict-overrides` regression detector.
//!
//! Instead the test drives `run_index` library-side, but goes through
//! the SAME [`IndexArgs`] clap parser the binary uses and the SAME
//! [`IndexArgs::apply_to`] field-translation helper +
//! [`index_error_exit_code`] exit-code mapping `main.rs` consults.
//! Every clap-binding / field-mapping / exit-code-mapping seam in the
//! production code path is exercised:
//!
//!   1. `IndexArgs::try_parse_from([..., "--strict-overrides"])` — proves
//!      the flag binds to `IndexArgs.strict_overrides`.
//!   2. `args.apply_to(&mut config)` — proves the field propagates to
//!      `IndexConfig.strict_overrides`.
//!   3. `run_index` is then driven with a [`LenientBackend`] stub plus a
//!      [`CapturingCollector`] override that mirrors the production
//!      [`StrictCollector`] / [`PermissiveCollector`] policy. The
//!      pipeline `(None, true) => StrictCollector` instantiation arm is
//!      covered by `cli_args::tests` + `override_warnings::tests` unit
//!      tests; substituting `CapturingCollector::new_strict()` here
//!      lets the test assert on the warning text without
//!      process-stderr capture acrobatics. The strict-mode contract
//!      this collector mirrors (echo + flip `has_errors()` on first
//!      emit) is unit-tested in `override_warnings.rs`.
//!   4. The resulting `IndexError` is mapped to an exit code via
//!      `index_error_exit_code` — the SAME helper `main.rs` consults.
//!      Strict mode must produce `IndexError::StrictOverridesFailed`
//!      (exit code 4); permissive mode must succeed (exit code 0).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{index_error_exit_code, run_index, IndexArgs, IndexConfig, IndexError};
use atlas_engine::testing::LenientBackend;
use atlas_engine::{CapturingCollector, OverrideWarningCollector};
use atlas_llm::LlmFingerprint;
use clap::Parser;
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0x77u8; 32],
        ontology_sha: [0x78u8; 32],
        model_id: "phase6-pr4-backend".into(),
        backend_version: "v-phase6-pr4".into(),
    }
}

/// Minimal clap harness mirroring `atlas`'s top-level
/// `Command::Index(IndexArgs)` shape so `try_parse_from` reads the
/// same argv the binary parses.
#[derive(Debug, Parser)]
#[command(name = "atlas")]
struct AtlasCli {
    #[command(subcommand)]
    command: AtlasCmd,
}

#[derive(Debug, clap::Subcommand)]
enum AtlasCmd {
    Index(IndexArgs),
}

fn parse_index_args(argv: &[&str]) -> IndexArgs {
    let parsed = AtlasCli::try_parse_from(argv)
        .unwrap_or_else(|err| panic!("clap parse failed for {argv:?}: {err}"));
    match parsed.command {
        AtlasCmd::Index(args) => args,
    }
}

/// Captures the result of a `run_index` invocation in a shape mirroring
/// `std::process::Output`. `status` carries the raw `Result` so the
/// test can assert on the specific `IndexError` variant; `stderr`
/// carries the captured warning text from the `CapturingCollector`;
/// `exit_code` is the value `main.rs` would have produced (via the
/// shared `index_error_exit_code` helper).
struct RunOutput {
    status: Result<atlas_cli::IndexSummary, IndexError>,
    stderr: String,
    exit_code: u8,
}

impl RunOutput {
    fn stderr(&self) -> &str {
        &self.stderr
    }
    fn status_code(&self) -> u8 {
        self.exit_code
    }
}

/// Drive `run_index` against `root` with no `--strict-overrides`
/// (permissive default). Returns the captured warning text plus the
/// exit code the CLI would emit.
fn run_atlas_index(root: &Path) -> RunOutput {
    run_atlas_index_with_args(root, &[])
}

/// Drive `run_index` against `root` with `extra_args` (typically
/// `["--strict-overrides"]` for the strict-mode arm). The argv is fed
/// through `IndexArgs::try_parse_from` and the resulting args are
/// translated to `IndexConfig` via `IndexArgs::apply_to` — i.e. the
/// exact same path `main.rs` uses.
fn run_atlas_index_with_args(root: &Path, extra_args: &[&str]) -> RunOutput {
    // Build the argv: `atlas index <root> --no-budget --no-gitignore <extra>`.
    // `--no-budget` mirrors the test-harness need (no real budget); the
    // binary uses `--budget <N>` in production. `--no-gitignore` mirrors
    // existing in-process integration tests that root Atlas at tempdirs.
    let root_str = root.to_str().expect("tempdir path is UTF-8");
    let mut argv: Vec<&str> = vec!["atlas", "index", root_str, "--no-budget", "--no-gitignore"];
    argv.extend_from_slice(extra_args);

    // Step 1 of the contract: clap parses the argv, binding
    // `--strict-overrides` (when present) to `IndexArgs.strict_overrides`.
    let args = parse_index_args(&argv);

    // Build the base `IndexConfig` the way `main.rs` does.
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.fingerprint_override = Some(fingerprint());

    // Step 2 of the contract: `apply_to` is the SAME helper `main.rs`
    // uses — sets `strict_overrides`, `respect_gitignore`, etc. The
    // test does NOT touch these fields directly.
    args.apply_to(&mut config);

    // Pivot of the contract: assert `apply_to` propagated the
    // `--strict-overrides` flag end-to-end. If the binding ever breaks
    // (e.g., someone renames the field in `IndexArgs` but forgets to
    // update `apply_to`), this assertion fires before run_index does.
    assert_eq!(
        config.strict_overrides,
        extra_args.contains(&"--strict-overrides"),
        "IndexArgs.strict_overrides binding broken: extra_args={extra_args:?}, \
         config.strict_overrides={}",
        config.strict_overrides
    );

    // Step 3 of the contract: substitute a `CapturingCollector` for
    // the in-process StrictCollector / PermissiveCollector. The
    // capturing collector mirrors the production strict/permissive
    // contract verbatim (echo + flip `has_errors()` on first emit for
    // strict; record-only for permissive). The
    // `pipeline.rs:(None, true) => StrictCollector` instantiation arm
    // is unit-tested in `cli_args::tests::exit_code_mapping_is_stable`
    // + `override_warnings::tests::strict_collector_sets_errors_on_emit`;
    // installing a capturing collector here lets the test assert on
    // the warning text without process-stderr capture.
    let collector: Arc<CapturingCollector> = Arc::new(if config.strict_overrides {
        CapturingCollector::new_strict()
    } else {
        CapturingCollector::new_permissive()
    });
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
    // Step 4 of the contract: map IndexError to exit code via the
    // SAME helper `main.rs` consults. No inline `match` on
    // `IndexError` in the test.
    let exit_code = match &status {
        Ok(_) => 0,
        Err(err) => index_error_exit_code(err),
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
           reason: \"forces EdgesOverrideUnknownKind\"\n",
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
    assert_eq!(
        output.status_code(),
        4,
        "strict mode must exit 4 (StrictOverridesFailed) when the warning fires"
    );
    assert!(matches!(
        output.status,
        Err(IndexError::StrictOverridesFailed(_))
    ));
}

// =============================================================
// Variant 2: EdgesOverrideUnknownKind
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
    assert_eq!(output.status_code(), 4, "strict mode must exit 4");
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
    assert_eq!(output.status_code(), 4, "strict mode must exit 4");
    assert!(matches!(
        output.status,
        Err(IndexError::StrictOverridesFailed(_))
    ));
}
