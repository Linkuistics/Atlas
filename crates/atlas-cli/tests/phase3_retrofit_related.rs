//! Phase 3 PR-5 sweep test — `related-components.yaml` cache-path retrofit.
//!
//! Acceptance criteria (plan §4 PR-5, §5 row):
//!
//! (a) After `run_index` completes, `<root>/.atlas/cache/related-components.yaml`
//!     exists and is non-empty (parseable as `RelatedComponentsFile`).
//! (b) The pre-PR-5 top-level `related-components.yaml` (sibling of
//!     `components.yaml` at the editorial tier) does **not** exist.
//!     I.e. nothing writes to the old non-cache location.
//!
//! The test uses the `tiny` fixture (two-crate Rust workspace).
//!
//! Fixture-build boilerplate (`materialise_fixture`, `base_config`,
//! `run_with`, `LenientBackend`) lives in the shared
//! `tests/common/sweep_support.rs` module — see Phase 4 PR-6.

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::run_index;
use component_ontology::RelatedComponentsFile;

mod common;
use common::sweep_support::*;

// ---------------------------------------------------------------------------
// PR-5 sweep test
// ---------------------------------------------------------------------------

/// PR-5 acceptance: after `run_index`, the cache-subdir path is populated and
/// the old top-level path does not exist.
#[test]
fn related_components_written_to_cache_subdir_not_top_level() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    // (a) cache/related-components.yaml must exist and be parseable.
    let cache_path = config.output_dir.join("cache/related-components.yaml");
    assert!(
        cache_path.exists(),
        "PR-5: expected cache/related-components.yaml at {} but it does not exist",
        cache_path.display()
    );
    let bytes = std::fs::read(&cache_path).expect("failed to read cache/related-components.yaml");
    assert!(
        !bytes.is_empty(),
        "PR-5: cache/related-components.yaml must not be empty"
    );
    let _parsed: RelatedComponentsFile = serde_yaml::from_slice(&bytes)
        .expect("cache/related-components.yaml must parse as RelatedComponentsFile");

    // (b) The old top-level path must NOT exist.
    let old_path = config.output_dir.join("related-components.yaml");
    assert!(
        !old_path.exists(),
        "PR-5: old top-level related-components.yaml must not exist after PR-5 retrofit; \
         found at {}",
        old_path.display()
    );
}

/// PR-5 acceptance: a dry-run must not write either path.
#[test]
fn dry_run_writes_neither_old_nor_new_path() {
    let tmp = materialise_fixture();
    let mut config = base_config(tmp.path());
    config.dry_run = true;

    // `run_with` panics on error, but a dry-run is a normal success path —
    // so we still drive the pipeline through it.
    run_index(
        &config,
        LenientBackend::new(sweep_fingerprint()),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("dry-run must not error");

    let cache_path = config.output_dir.join("cache/related-components.yaml");
    assert!(
        !cache_path.exists(),
        "PR-5: dry-run must not write cache/related-components.yaml; found at {}",
        cache_path.display()
    );

    let old_path = config.output_dir.join("related-components.yaml");
    assert!(
        !old_path.exists(),
        "PR-5: dry-run must not write old top-level related-components.yaml; found at {}",
        old_path.display()
    );
}

/// PR-5 acceptance: a second `run_index` call (re-run) still reads from the
/// cache path and produces byte-identical output.
#[test]
fn second_run_reads_from_cache_path_and_is_byte_identical() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    // First run.
    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    let cache_path = config.output_dir.join("cache/related-components.yaml");
    let first_bytes = std::fs::read(&cache_path)
        .expect("cache/related-components.yaml must exist after first run");

    // Second run.
    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    let second_bytes = std::fs::read(&cache_path)
        .expect("cache/related-components.yaml must exist after second run");

    assert_eq!(
        first_bytes, second_bytes,
        "PR-5: cache/related-components.yaml must be byte-identical on re-run"
    );
}
