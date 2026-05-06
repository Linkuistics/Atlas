//! Multi-root Workspace tests (PR-3).
//!
//! Three guarantees:
//!
//! 1. **Two-root union with stable ids.** A workspace with two
//!    Cargo-crate roots assembles `all_components` to the union of
//!    both, and the per-crate ids are deterministic (rerunning against
//!    the same fixture produces the same id set).
//! 2. **Single-root no-op cache hit.** A second pass over the same
//!    workspace makes zero LLM calls. The `TestBackend` panics on any
//!    unrecognised request — for the all-deterministic Cargo-only
//!    fixture this is the same guarantee, but the explicit
//!    `call_count == 0` assertion on the second pass is the canonical
//!    cache-hit guard called out in plan §6.
//! 3. **Two-root no-op cache hit.** Same guarantee for a two-root
//!    workspace. The per-root walks are independent, so this exercises
//!    the multi-root code paths without leaking cache state across
//!    roots.

use std::path::Path;
use std::sync::Arc;

use atlas_engine::{all_components, seed_filesystem, AtlasDatabase};
use atlas_llm::{LlmFingerprint, TestBackend};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [1u8; 32],
        ontology_sha: [2u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v0".into(),
    }
}

fn write_cargo_lib(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
}

#[test]
fn two_root_workspace_unions_components_with_stable_ids() {
    // Each tempdir is a separate root. Atlas indexes both as a single
    // multi-root workspace; the result tree is the union of components
    // discovered under each root.
    let primary = TempDir::new().unwrap();
    let peer = TempDir::new().unwrap();
    write_cargo_lib(primary.path(), "alpha");
    write_cargo_lib(peer.path(), "beta");

    let backend: Arc<dyn atlas_llm::LlmBackend> = Arc::new(TestBackend::new());
    let roots = vec![primary.path().to_path_buf(), peer.path().to_path_buf()];
    let mut db = AtlasDatabase::new(backend, roots.clone(), fingerprint());
    seed_filesystem(&mut db, &roots, false).expect("seed multi-root fixture");

    let tree = all_components(&db);
    let mut ids: Vec<&str> = tree.iter().map(|c| c.id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["alpha", "beta"],
        "both roots' Cargo libraries should appear in the unioned tree"
    );
    for c in tree.iter() {
        assert_eq!(c.kind, "rust-library");
        assert!(!c.deleted);
        assert_eq!(c.path_segments.len(), 1);
    }

    // Run again with the same shapes; the id set must be identical
    // (this is the "stable ids" half of the acceptance criterion —
    // rerunning over the same fixture yields the same ids).
    let backend2: Arc<dyn atlas_llm::LlmBackend> = Arc::new(TestBackend::new());
    let mut db2 = AtlasDatabase::new(backend2, roots.clone(), fingerprint());
    seed_filesystem(&mut db2, &roots, false).expect("seed multi-root fixture (2nd time)");
    let tree2 = all_components(&db2);
    let mut ids2: Vec<&str> = tree2.iter().map(|c| c.id.as_str()).collect();
    ids2.sort();
    assert_eq!(ids, ids2, "ids must be stable across a fresh re-seed");
}

#[test]
fn single_root_noop_rerun_makes_zero_llm_calls() {
    // The cache-hit-on-no-op-rerun guard from plan §6: a single-root
    // workspace with all-deterministic components classifies via the
    // L3 heuristic table; no LLM call is dispatched at all.
    // `call_count` is the canonical engine-level guard.
    let tmp = TempDir::new().unwrap();
    write_cargo_lib(tmp.path(), "alpha");

    let backend: Arc<dyn atlas_llm::LlmBackend> = Arc::new(TestBackend::new());
    let mut db = AtlasDatabase::new(backend, vec![tmp.path().to_path_buf()], fingerprint());
    seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();

    let _first = all_components(&db);
    let after_first = db.llm_cache().call_count();
    let _second = all_components(&db);
    let after_second = db.llm_cache().call_count();
    assert_eq!(
        after_second, after_first,
        "second tree assembly over unchanged inputs must not dispatch any \
         additional LLM call (cache-hit invariant)"
    );
    // Cargo-only fixtures classify deterministically; both passes
    // should be zero.
    assert_eq!(
        after_second, 0,
        "Cargo-only fixture must classify deterministically — any LLM \
         call indicates a regression in the heuristic short-circuit"
    );
}

#[test]
fn two_root_noop_rerun_makes_zero_llm_calls() {
    // Same as the single-root case, generalised to two roots. The
    // per-root walks are independent; a missed `workspace.root()`
    // callsite anywhere in the engine would re-execute the LLM-driven
    // path on the second pass and increment `call_count`. The plan §6
    // risk row "PR-3 breaks the Salsa cache hit on the in-tree
    // fixtures due to a missed `workspace.root()` callsite" calls this
    // out as the canonical guard.
    let primary = TempDir::new().unwrap();
    let peer = TempDir::new().unwrap();
    write_cargo_lib(primary.path(), "alpha");
    write_cargo_lib(peer.path(), "beta");

    let backend: Arc<dyn atlas_llm::LlmBackend> = Arc::new(TestBackend::new());
    let roots = vec![primary.path().to_path_buf(), peer.path().to_path_buf()];
    let mut db = AtlasDatabase::new(backend, roots.clone(), fingerprint());
    seed_filesystem(&mut db, &roots, false).unwrap();

    let _first = all_components(&db);
    let after_first = db.llm_cache().call_count();
    let _second = all_components(&db);
    let after_second = db.llm_cache().call_count();
    assert_eq!(
        after_second, after_first,
        "second multi-root tree assembly must not dispatch any \
         additional LLM call (cache-hit invariant under multi-root)"
    );
    assert_eq!(
        after_second, 0,
        "Cargo-only multi-root fixture must classify deterministically; \
         any LLM call indicates a missed `workspace.roots()` callsite"
    );
}
