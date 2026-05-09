//! Shared helpers for the Phase 3 retrofit sweep tests
//! (`phase3_retrofit_{surfaces,component,components,related}.rs`).
//!
//! Each of those four files independently shipped ~100 LoC of identical
//! fixture-build boilerplate (canned-response backend, tiny-fixture copy
//! into a `TempDir`, `IndexConfig` builder, `run_index` driver). Phase 4
//! PR-6 consolidates that boilerplate here.
//!
//! ## What lives in this module
//!
//! - [`sweep_fingerprint`] — a fixed `LlmFingerprint` shared by all four
//!   sweep tests. The tests only need the fingerprint to be stable across
//!   the two runs they perform within a single test; they do not assert
//!   on the fingerprint's bytes, so a single canonical value is enough.
//!
//! - [`tiny_fixture_root`] / [`copy_dir_all`] / [`materialise_fixture`] —
//!   Materialise the on-disk `tests/fixtures/tiny/` workspace into a
//!   fresh `TempDir`.
//!
//! - [`base_config`] — Construct an `IndexConfig` rooted at a `TempDir`
//!   path with `output_dir = root/.atlas`, `respect_gitignore = false`,
//!   and `fingerprint_override = Some(sweep_fingerprint())`.
//!
//! - [`run_with`] — Drive `run_index` with the supplied config and
//!   backend through a no-op stderr progress reporter.
//!
//! - [`LenientBackend`] (re-export) — The canonical permissive
//!   canned-response backend lives in `atlas_engine::testing` (Phase 4
//!   PR-1). All four sweep tests now use it instead of file-local
//!   `SweepBackend` copies; the `SweepBackend` structs are deleted as
//!   part of this PR.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_llm::{LlmBackend, LlmFingerprint};
use tempfile::TempDir;

pub use atlas_engine::testing::LenientBackend;

/// Fixed sweep-test fingerprint.
///
/// All four `phase3_retrofit_*.rs` tests share this fingerprint via
/// `IndexConfig::fingerprint_override`. The bytes are arbitrary; the
/// tests only need stability across a single test's two runs (for
/// the byte-identical re-run assertions), not any specific value.
pub fn sweep_fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0xA2u8; 32],
        ontology_sha: [0xA3u8; 32],
        model_id: "phase3-sweep-backend".into(),
        backend_version: "v-phase3-sweep".into(),
    }
}

/// Path to the on-disk `tests/fixtures/tiny/` two-crate Rust workspace.
pub fn tiny_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tiny")
}

/// Recursively copy `src` into `dst`, creating `dst` if missing.
pub fn copy_dir_all(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir_all(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// Materialise the tiny fixture into a fresh `TempDir` and return it.
pub fn materialise_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    copy_dir_all(&tiny_fixture_root(), tmp.path());
    tmp
}

/// Build an `IndexConfig` rooted at `root` with the canonical sweep-test
/// settings: `output_dir = root/.atlas`, gitignore disabled,
/// `fingerprint_override = Some(sweep_fingerprint())`.
pub fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(sweep_fingerprint());
    config
}

/// Drive `run_index` with a no-op stderr progress reporter and panic on
/// any error. Used by the sweep tests to exercise the full pipeline.
pub fn run_with(config: &IndexConfig, backend: Arc<dyn LlmBackend>) {
    run_index(
        config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index must succeed");
}
