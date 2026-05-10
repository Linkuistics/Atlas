//! PR-10 acceptance tests for the persistent content-addressed cache.
//!
//! Three behaviours are guaranteed by the brief:
//!
//! 1. A no-op re-run after dropping in-memory state (i.e., a fresh
//!    process invocation, simulated here by running `run_index` twice
//!    with fresh backends) makes zero LLM calls — every entry hits
//!    the persistent layer.
//! 2. A single source-file content change invalidates only the entries
//!    whose fingerprint cited that file. Other entries still hit the
//!    persistent cache.
//! 3. Deleting `<output>/.atlas/cache/` forces a full rerun (every
//!    entry misses) and the cache rebuilds.

use std::path::{Path, PathBuf};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_engine::testing::LenientBackend;
use atlas_llm::{LlmFingerprint, PromptId};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [1u8; 32],
        ontology_sha: [2u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v-test".into(),
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

fn cold_run_baseline(config: &IndexConfig) -> usize {
    let backend = LenientBackend::new(fingerprint());
    run_index(
        config,
        backend.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("cold run");
    backend.call_count()
}

#[test]
fn fresh_process_re_run_hits_persistent_cache_for_every_entry() {
    // Acceptance criterion #1: a no-op re-run after dropping the
    // in-memory state makes zero LLM calls. Each `run_index` call
    // constructs a fresh `AtlasDatabase` with a fresh
    // `LlmResponseCache`, so the second call's hits must come from
    // the persistent layer alone.
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    let cold_calls = cold_run_baseline(&config);
    assert!(
        cold_calls > 0,
        "cold run must exercise the backend for the test to be meaningful"
    );

    let backend2 = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend2.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("warm run");
    assert_eq!(
        backend2.call_count(),
        0,
        "fresh-process re-run must hit the persistent cache for every entry; \
         actual calls: {:?}",
        backend2.calls_with_inputs()
    );

    // The on-disk layout should match design §5.4.
    let cache_root = config.output_dir.join("cache");
    assert!(
        cache_root.exists(),
        "persistent cache root must exist after a successful run"
    );
}

#[test]
fn single_file_content_change_invalidates_only_affected_entries() {
    // Acceptance criterion #2: editing one source file invalidates
    // only the entries whose fingerprint cited that file. Entries
    // for the unrelated component still hit the persistent cache.
    //
    // The tiny fixture has `mylib` (rust-library) and `mycli`
    // (rust-bin). Editing `mylib/src/lib.rs` reshapes mylib's
    // `path_segments[0].content_sha`, which the L5 fingerprint
    // contributes. The L6 batch fingerprint contributes every
    // component's segment shas, so the L6 entry invalidates too.
    // mycli's L5 entry must still hit the persistent cache.
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    let _ = cold_run_baseline(&config);

    // Mutate one file. seed_filesystem on the next run picks up
    // the new bytes via Salsa input churn.
    std::fs::write(tmp.path().join("mylib/src/lib.rs"), "// modified by test\n").unwrap();

    let backend2 = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend2.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("warm run with file change");

    let calls = backend2.calls_with_inputs();
    let surface_calls: Vec<&(PromptId, String)> = calls
        .iter()
        .filter(|(p, _)| *p == PromptId::Stage1Surface)
        .collect();
    let edge_calls: Vec<&(PromptId, String)> = calls
        .iter()
        .filter(|(p, _)| *p == PromptId::Stage2Edges)
        .collect();

    // The edited component (mylib) must have a fresh L5 call: its
    // content_sha changed, the fingerprint cited that sha, and
    // therefore the persistent cache misses. Assert by matching on
    // the request's `COMPONENT_ID` substring inside the canonical
    // inputs JSON.
    let mylib_surface_calls = surface_calls
        .iter()
        .filter(|(_, inputs)| inputs.contains("\"COMPONENT_ID\":\"mylib\""))
        .count();
    assert!(
        mylib_surface_calls >= 1,
        "mylib's Stage1Surface entry must miss after its source file changed; \
         surface calls: {surface_calls:?}"
    );

    // mycli's L5 entry was NOT cited by the change — its
    // path_segments[0].content_sha is unaffected — so the
    // persistent cache must still satisfy its lookup. The "only
    // affected entries miss" property reduces to: no fresh call
    // for an unrelated component.
    let mycli_surface_calls = surface_calls
        .iter()
        .filter(|(_, inputs)| inputs.contains("\"COMPONENT_ID\":\"mycli\""))
        .count();
    assert_eq!(
        mycli_surface_calls, 0,
        "mycli's Stage1Surface entry must hit the persistent cache (its content sha is \
         unaffected by the mylib edit); surface calls: {surface_calls:?}"
    );

    // L6 batch fingerprint contributes every component's segment
    // shas, so it invalidates whenever any component's segments
    // change. Assert >=1 to document the propagation; the exact
    // count depends on whether L8 sub-carve adds new candidate
    // directories on this run.
    assert!(
        !edge_calls.is_empty(),
        "L6 batch must miss when any participating component's content sha changes; \
         edge calls: {edge_calls:?}"
    );
}

#[test]
fn deleting_cache_directory_forces_full_rerun() {
    // Acceptance criterion #3: deleting `<output>/.atlas/cache/`
    // forces a full re-run; every entry misses and the cache
    // rebuilds.
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    let cold_calls = cold_run_baseline(&config);
    assert!(cold_calls > 0);

    // Delete the persistent cache root only — leave the YAMLs and
    // the rest of `.atlas/` alone. The next run starts with no
    // persistent state and rebuilds.
    std::fs::remove_dir_all(config.output_dir.join("cache")).unwrap();

    let backend2 = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend2.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("rebuild run");
    assert_eq!(
        backend2.call_count(),
        cold_calls,
        "after deleting the persistent cache, every entry must miss again; \
         cold={cold_calls} rebuild={}",
        backend2.call_count(),
    );
    assert!(
        config.output_dir.join("cache").exists(),
        "the rebuild run must repopulate <output>/.atlas/cache/"
    );

    // A subsequent run is back to all-hits.
    let backend3 = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend3.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("third run");
    assert_eq!(
        backend3.call_count(),
        0,
        "third run after rebuild must be all-hits"
    );
}
