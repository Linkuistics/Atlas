//! Path-dep root expansion to fixed point (PR-4 of Atlas vNext Phase 1).
//!
//! Five guarantees:
//!
//! 1. **Single-root, no escaping path-deps.** A workspace whose only
//!    Cargo crates are inside the primary root yields `vec![primary]`.
//! 2. **Two-root path-dep.** A primary-root crate with a path-dep
//!    pointing at a sibling tree adds the sibling's crate root as a
//!    peer.
//! 3. **Two-root with workspace at the target.** When the path-dep
//!    target is a workspace member, the walk-up locates the
//!    `[workspace]` manifest and the peer root is the workspace
//!    directory (not the crate directory).
//! 4. **Cycle.** A `crate-a → crate-b → crate-a` cycle terminates
//!    finitely; the function returns a finite `Vec` rather than
//!    looping. (Stderr capture is left to the binary; the test asserts
//!    on the absence of an infinite loop and on the result shape.)
//! 5. **Path-dep that doesn't exist.** A `path = "../missing-crate"`
//!    pointing at a non-existent directory is silently skipped — the
//!    fixed-point walk is intentionally lenient about uncloned
//!    sibling repos.

use std::path::{Path, PathBuf};

use atlas_engine::expand_roots;
use tempfile::TempDir;

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A self-contained Cargo crate at `dir` with optional path-deps.
fn write_cargo_crate(dir: &Path, name: &str, path_deps: &[(&str, &str)]) {
    let mut manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n"
    );
    if !path_deps.is_empty() {
        manifest.push_str("\n[dependencies]\n");
        for (dep_name, dep_path) in path_deps {
            manifest.push_str(&format!("{dep_name} = {{ path = \"{dep_path}\" }}\n"));
        }
    }
    write(&dir.join("Cargo.toml"), &manifest);
    write(&dir.join("src/lib.rs"), "// lib\n");
}

#[test]
fn single_root_no_path_deps() {
    let primary = TempDir::new().unwrap();
    write_cargo_crate(&primary.path().join("crate-a"), "crate-a", &[]);
    write_cargo_crate(&primary.path().join("crate-b"), "crate-b", &[]);

    let roots = expand_roots(primary.path()).expect("expand_roots succeeds");
    let expected = primary.path().canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![expected],
        "single-root fixture must return only the primary root"
    );
}

#[test]
fn two_root_path_dep() {
    // The primary root holds `crate-a` which path-deps `crate-b` in a
    // sibling tree. `crate-b` is a standalone crate (no enclosing
    // workspace); the walk-up returns the crate directory itself.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external/crate-b")],
    );
    write_cargo_crate(&external.join("crate-b"), "crate-b", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let crate_b_canonical = external.join("crate-b").canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![primary_canonical, crate_b_canonical],
        "primary then crate-b's directory (no enclosing workspace)"
    );
}

#[test]
fn two_root_with_workspace_at_target() {
    // The external tree is a Cargo workspace; the path-dep target
    // (crate-b) is a workspace member. The walk-up MUST return the
    // workspace's directory, not the member crate's directory.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external/crates/crate-b")],
    );
    write(
        &external.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/crate-b\"]\nresolver = \"2\"\n",
    );
    write_cargo_crate(&external.join("crates/crate-b"), "crate-b", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let workspace_canonical = external.canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![primary_canonical, workspace_canonical],
        "the peer root is the [workspace] manifest's directory, not the member's"
    );
}

#[test]
fn cycle_terminates_with_warning() {
    // crate-a (primary) path-deps crate-b (external)
    // crate-b path-deps crate-c (back inside primary)
    //
    // The visited-set guard means the walk terminates after enumerating
    // both peer roots once (here just one peer, since crate-c is inside
    // the already-known primary). The test's load-bearing guarantee is
    // that the call returns at all — a cycle bug would loop forever.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external/crate-b")],
    );
    write_cargo_crate(
        &external.join("crate-b"),
        "crate-b",
        &[("crate-c", "../../primary/crate-c")],
    );
    // crate-c lives inside the primary tree; the fixed-point walk
    // notices and skips re-adding the primary as a peer.
    write_cargo_crate(&primary.join("crate-c"), "crate-c", &[]);

    let roots = expand_roots(&primary).expect("expand_roots terminates");
    assert_eq!(
        roots.len(),
        2,
        "primary + external/crate-b only; crate-c stays inside the primary's coverage"
    );
    let primary_canonical = primary.canonicalize().unwrap();
    let external_b_canonical = external.join("crate-b").canonicalize().unwrap();
    assert_eq!(roots[0], primary_canonical);
    assert_eq!(roots[1], external_b_canonical);
}

#[test]
fn path_dep_to_missing_crate_is_skipped() {
    // A manifest may declare a `path = "../sibling"` whose target is
    // not yet checked out (worktree of a multi-repo monorepo, partial
    // git clone). PR-4 treats this as data, not error: the path-dep
    // is skipped and the rest of the walk proceeds.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("missing", "../../external/missing-crate")],
    );
    // crate-b (real) path-deps a real external sibling so the walk
    // still produces a peer; this shows the missing path does not
    // poison the rest of the walk.
    write_cargo_crate(
        &primary.join("crate-b"),
        "crate-b",
        &[("real", "../../external/real-crate")],
    );
    write_cargo_crate(
        &parent.path().join("external/real-crate"),
        "real-crate",
        &[],
    );

    let roots = expand_roots(&primary).expect("expand_roots succeeds despite missing path-dep");

    let primary_canonical = primary.canonicalize().unwrap();
    let real_canonical = parent
        .path()
        .join("external/real-crate")
        .canonicalize()
        .unwrap();
    assert_eq!(roots, vec![primary_canonical, real_canonical]);
}

/// Sanity: path-deps that resolve back inside the primary root (a
/// natural shape for in-tree workspace members referenced by sibling
/// path) are not added as peer roots.
#[test]
fn path_dep_inside_primary_is_not_added_as_peer() {
    let primary = TempDir::new().unwrap();
    write_cargo_crate(
        &primary.path().join("crate-a"),
        "crate-a",
        &[("crate-b", "../crate-b")],
    );
    write_cargo_crate(&primary.path().join("crate-b"), "crate-b", &[]);

    let roots = expand_roots(primary.path()).expect("expand_roots succeeds");
    let expected = primary.path().canonicalize().unwrap();
    assert_eq!(roots, vec![expected]);
}

/// True cross-tree cycle: crate-b in `external-1` path-deps crate-c
/// in `external-2`, and crate-c path-deps crate-b. The fixed-point
/// walk discovers external-1 (via the primary's path-dep) and
/// external-2 (via crate-b's), and on revisiting external-2's
/// path-dep to crate-b the visited-set guard short-circuits — no
/// stack overflow, no infinite loop. The expected discovery order is
/// `[primary, external-1, external-2]`.
#[test]
fn cross_tree_cycle_terminates() {
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let ext1 = parent.path().join("external-1");
    let ext2 = parent.path().join("external-2");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external-1/crate-b")],
    );
    write_cargo_crate(
        &ext1.join("crate-b"),
        "crate-b",
        &[("crate-c", "../../external-2/crate-c")],
    );
    write_cargo_crate(
        &ext2.join("crate-c"),
        "crate-c",
        &[("crate-b", "../../external-1/crate-b")],
    );

    let roots = expand_roots(&primary).expect("expand_roots terminates on cross-tree cycle");
    assert_eq!(roots.len(), 3, "primary + two peers, no infinite loop");

    let primary_canonical = primary.canonicalize().unwrap();
    let ext1_canonical = ext1.join("crate-b").canonicalize().unwrap();
    let ext2_canonical = ext2.join("crate-c").canonicalize().unwrap();
    assert_eq!(roots[0], primary_canonical);
    assert_eq!(roots[1], ext1_canonical);
    assert_eq!(roots[2], ext2_canonical);
}

/// Convenience: a path-dep through a chain of intermediate peers
/// works (a → b → c, all in different trees) and produces all three
/// roots in discovery order.
#[test]
fn transitive_path_deps_reach_all_peers() {
    let parent = TempDir::new().unwrap();
    let primary: PathBuf = parent.path().join("primary");
    let mid = parent.path().join("mid");
    let far = parent.path().join("far");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../mid/crate-b")],
    );
    write_cargo_crate(
        &mid.join("crate-b"),
        "crate-b",
        &[("crate-c", "../../far/crate-c")],
    );
    write_cargo_crate(&far.join("crate-c"), "crate-c", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let mid_canonical = mid.join("crate-b").canonicalize().unwrap();
    let far_canonical = far.join("crate-c").canonicalize().unwrap();
    assert_eq!(roots, vec![primary_canonical, mid_canonical, far_canonical]);
}
