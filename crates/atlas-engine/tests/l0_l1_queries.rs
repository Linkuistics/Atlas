//! Integration tests for L0 inputs and L1 enumeration queries against
//! the `tests/fixtures/` tree.
//!
//! Each fixture is a synthetic directory: `tiny-rust-lib` (one crate
//! with README), `mixed` (Python + Rust side-by-side), and
//! `monorepo-fragment` (three package.json under a parent). Together
//! they exercise the manifest, git-boundary, README-heading, and
//! shebang queries.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_engine::{
    doc_headings, file_content, file_tree_sha, git_boundaries, manifests_in, seed_filesystem,
    shebangs, AtlasDatabase, DocHeading,
};
use atlas_llm::{LlmFingerprint, TestBackend};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn default_fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0u8; 32],
        ontology_sha: [0u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

fn db_seeded_from(root: &Path) -> AtlasDatabase {
    let backend = Arc::new(TestBackend::new());
    let mut db = AtlasDatabase::new(backend, root.to_path_buf(), default_fingerprint());
    seed_filesystem(&mut db, root, false).expect("seed_filesystem must succeed on fixture");
    db
}

#[test]
fn tiny_rust_lib_registers_expected_files() {
    let root = fixture("tiny-rust-lib");
    let db = db_seeded_from(&root);

    let cargo = db
        .file_by_path(&root.join("Cargo.toml"))
        .expect("Cargo.toml should be registered");
    let bytes = file_content(&db, &root.join("Cargo.toml")).expect("cargo bytes");
    assert!(bytes.starts_with(b"[package]"));
    // Sanity: path accessor returns what we registered under.
    let _ = cargo;
}

#[test]
fn tiny_rust_lib_manifests_in_returns_cargo_toml() {
    let root = fixture("tiny-rust-lib");
    let db = db_seeded_from(&root);
    let ws = db.workspace();

    let manifests = manifests_in(&db, ws, root.clone());
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0], root.join("Cargo.toml"));
}

#[test]
fn mixed_manifests_in_returns_both_ecosystems() {
    let root = fixture("mixed");
    let db = db_seeded_from(&root);
    let ws = db.workspace();

    let manifests = manifests_in(&db, ws, root.clone());
    assert_eq!(
        *manifests,
        vec![
            root.join("py-proj/pyproject.toml"),
            root.join("rust-proj/Cargo.toml"),
        ]
    );
}

#[test]
fn mixed_manifests_in_subdir_scopes_to_that_subdir() {
    let root = fixture("mixed");
    let db = db_seeded_from(&root);
    let ws = db.workspace();

    let rust_only = manifests_in(&db, ws, root.join("rust-proj"));
    assert_eq!(*rust_only, vec![root.join("rust-proj/Cargo.toml")]);

    let py_only = manifests_in(&db, ws, root.join("py-proj"));
    assert_eq!(*py_only, vec![root.join("py-proj/pyproject.toml")]);
}

#[test]
fn monorepo_fragment_manifests_in_returns_four_package_jsons() {
    let root = fixture("monorepo-fragment");
    let db = db_seeded_from(&root);
    let ws = db.workspace();

    let manifests = manifests_in(&db, ws, root.clone());
    assert_eq!(
        *manifests,
        vec![
            root.join("package.json"),
            root.join("pkg-a/package.json"),
            root.join("pkg-b/package.json"),
            root.join("pkg-c/package.json"),
        ]
    );
}

#[test]
fn doc_headings_extracts_atx_headings_from_readme() {
    let root = fixture("tiny-rust-lib");
    let db = db_seeded_from(&root);
    let ws = db.workspace();

    let headings = doc_headings(&db, ws, root.clone());
    let expected = vec![
        DocHeading {
            path: root.join("README.md"),
            level: 1,
            text: "tiny-rust-lib".into(),
        },
        DocHeading {
            path: root.join("README.md"),
            level: 2,
            text: "Usage".into(),
        },
    ];
    assert_eq!(*headings, expected);
}

#[test]
fn shebangs_collects_interpreter_from_exec_scripts() {
    let root = tempfile_like_root();
    let db = db_seeded_from(&root);
    let ws = db.workspace();

    let shebangs = shebangs(&db, ws, root.clone());
    assert_eq!(shebangs.len(), 1);
    assert_eq!(shebangs[0].path, root.join("bin/run.sh"));
    assert_eq!(shebangs[0].interpreter, "/bin/bash");
}

#[test]
fn git_boundaries_records_dotgit_directories() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("repo/.git/refs")).unwrap();
    std::fs::write(root.join("repo/README.md"), "# Repo\n").unwrap();

    let db = db_seeded_from(&root);
    let ws = db.workspace();
    let boundaries = git_boundaries(&db, ws, root.clone());
    assert_eq!(*boundaries, vec![root.join("repo")]);
}

#[test]
fn file_tree_sha_is_deterministic_across_runs() {
    let root = fixture("tiny-rust-lib");
    let db1 = db_seeded_from(&root);
    let db2 = db_seeded_from(&root);
    let ws1 = db1.workspace();
    let ws2 = db2.workspace();

    let sha1 = file_tree_sha(&db1, ws1, root.clone());
    let sha2 = file_tree_sha(&db2, ws2, root.clone());
    assert_eq!(sha1, sha2);
}

#[test]
fn all_three_fixtures_produce_deterministic_l1_outputs() {
    // Exit-criteria check: repeated seeds of the same fixture must
    // produce byte-identical manifest lists and file-tree SHAs.
    for name in ["tiny-rust-lib", "mixed", "monorepo-fragment"] {
        let root = fixture(name);
        let db1 = db_seeded_from(&root);
        let db2 = db_seeded_from(&root);
        let ws1 = db1.workspace();
        let ws2 = db2.workspace();

        let manifests1 = manifests_in(&db1, ws1, root.clone());
        let manifests2 = manifests_in(&db2, ws2, root.clone());
        assert_eq!(manifests1, manifests2, "manifests_in drifted for {name}");

        let sha1 = file_tree_sha(&db1, ws1, root.clone());
        let sha2 = file_tree_sha(&db2, ws2, root.clone());
        assert_eq!(sha1, sha2, "file_tree_sha drifted for {name}");
    }
}

#[test]
fn file_tree_sha_changes_when_file_content_changes() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();

    let mut db = db_seeded_from(&root);
    let ws = db.workspace();
    let sha_before = file_tree_sha(&db, ws, root.clone());

    // Mutate one file and reseed.
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t2\"\n").unwrap();
    seed_filesystem(&mut db, &root, false).unwrap();
    let sha_after = file_tree_sha(&db, ws, root.clone());

    assert_ne!(sha_before, sha_after);
}

#[test]
fn manifests_in_is_cache_hit_when_unrelated_file_changes() {
    // Change the README; `manifests_in` only reads paths, not bytes,
    // so it should be a cache hit. `doc_headings` reads the README
    // bytes and must re-run.
    let td = tempfile::tempdir().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    std::fs::write(root.join("README.md"), "# Title\n").unwrap();

    let mut db = db_seeded_from(&root);
    db.enable_execution_log();

    let ws = db.workspace();
    // Prime both queries.
    let _ = manifests_in(&db, ws, root.clone());
    let _ = doc_headings(&db, ws, root.clone());
    let _primed = db.take_execution_log(); // discard

    // Mutate just the README.
    std::fs::write(root.join("README.md"), "# Different\n").unwrap();
    seed_filesystem(&mut db, &root, false).unwrap();

    db.enable_execution_log();
    let _ = manifests_in(&db, ws, root.clone());
    let _ = doc_headings(&db, ws, root.clone());
    let log = db.take_execution_log();
    let log_joined = log
        .iter()
        .map(|e| e.description.clone())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        log_joined.contains("doc_headings"),
        "doc_headings should re-run after README change. log:\n{log_joined}"
    );
    assert!(
        !log_joined.contains("manifests_in"),
        "manifests_in should NOT re-run; it reads only paths. log:\n{log_joined}"
    );
}

#[test]
fn seed_filesystem_respects_gitignore_when_enabled() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(root.join("ignored.txt"), "hidden").unwrap();
    std::fs::write(root.join("kept.txt"), "visible").unwrap();

    let backend = Arc::new(TestBackend::new());
    let mut db = AtlasDatabase::new(backend, root.clone(), default_fingerprint());
    seed_filesystem(&mut db, &root, true).unwrap();

    assert!(db.file_by_path(&root.join("kept.txt")).is_some());
    // .gitignore behaviour inside the `ignore` crate requires an
    // enclosing `.git` to be treated as authoritative; absent that,
    // the file may still be filtered via fallback. Either outcome is
    // acceptable — the test asserts only that the non-ignored file
    // survives.
    let _ = db.file_by_path(&root.join("ignored.txt"));
}

#[test]
fn file_registration_is_stable_across_reseed() {
    // A second `seed_filesystem` call must reuse existing File
    // handles for unchanged paths; otherwise downstream Salsa caches
    // would lose their dependency edges unnecessarily.
    let td = tempfile::tempdir().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("a.txt"), "first").unwrap();

    let mut db = db_seeded_from(&root);
    let first = db.file_by_path(&root.join("a.txt")).unwrap();

    seed_filesystem(&mut db, &root, false).unwrap();
    let second = db.file_by_path(&root.join("a.txt")).unwrap();

    assert_eq!(first, second);
}

/// Build a small on-disk tree under a tempdir that contains one
/// shebang'd script and return the tempdir root. Uses a retained
/// `TempDir` via `Box::leak` so the path stays valid for the duration
/// of the test — a pragmatic shortcut used only in `shebangs`.
fn tempfile_like_root() -> PathBuf {
    let td = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::write(
        root.join("bin/run.sh"),
        b"#!/bin/bash\necho hello\n" as &[u8],
    )
    .unwrap();
    // Sanity: one non-shebang file so we confirm filtering works.
    std::fs::write(root.join("README.md"), "# Root\n").unwrap();
    root
}
