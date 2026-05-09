//! Phase 3 PR-2 sweep test — per-component surfaces.yaml cache-path retrofit.
//!
//! Acceptance criteria (plan §4 PR-2, §5 row):
//!
//! (a) After `run_index`, every component directory contains a non-empty
//!     `<component>/.atlas/cache/surfaces.yaml` (parseable as `SurfacesFile`).
//! (b) No pre-PR-2 `surfaces.yaml` exists directly under `<component>/.atlas/`
//!     (i.e. without the `cache/` sub-directory).
//! (c) Cache-hit re-run: two consecutive runs with identical inputs produce
//!     byte-identical `cache/surfaces.yaml` files (fingerprint-equality, not
//!     mtime). This verifies the path move did not break Phase 1's L5 cache
//!     invariant.
//! (d) `.atlas/.gitignore` exists at each component scope after the run (PR-1
//!     gitignore session wired by `write_per_component_files`).
//!
//! The test uses the `tiny` fixture (two-crate Rust workspace: `mylib` + `mycli`).
//! A canned-response backend avoids any real LLM calls.
//!
//! Fixture-build boilerplate (`materialise_fixture`, `base_config`,
//! `run_with`, the canned-response backend) lives in the shared
//! `tests/common/sweep_support.rs` module — see Phase 4 PR-6.

use std::path::{Path, PathBuf};

use atlas_index::{SurfacesFile, SURFACES_SCHEMA_VERSION};

mod common;
use common::sweep_support::*;

// ---------------------------------------------------------------------------
// Walk helpers (test-specific — only PR-2 cares about old-path surfaces.yaml).
// ---------------------------------------------------------------------------

/// Recursively collect all files named `surfaces.yaml` whose path does NOT
/// include a `cache` path component. These are "old-path" violations.
fn find_surfaces_yaml_outside_cache(root: &Path) -> Vec<PathBuf> {
    let mut violators = Vec::new();
    walk_for_old_surfaces(root, &mut violators);
    violators
}

fn walk_for_old_surfaces(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_for_old_surfaces(&path, out);
        } else {
            let is_surfaces = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "surfaces.yaml")
                .unwrap_or(false);
            if is_surfaces {
                // Permitted only when one of the path components is `cache`.
                let in_cache = path.components().any(|c| {
                    c.as_os_str()
                        .to_str()
                        .map(|s| s == "cache")
                        .unwrap_or(false)
                });
                if !in_cache {
                    out.push(path);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AC(a) + AC(b): cache/surfaces.yaml populated, no old-path file.
// ---------------------------------------------------------------------------

#[test]
fn cache_surfaces_yaml_written_for_each_component_and_no_old_path() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    // AC(a): every component directory must contain a non-empty
    // cache/surfaces.yaml that parses as SurfacesFile.
    for name in ["mylib", "mycli"] {
        let cache_path = tmp.path().join(name).join(".atlas/cache/surfaces.yaml");
        assert!(
            cache_path.exists(),
            "PR-2: expected cache/surfaces.yaml for `{name}` at {} — does not exist",
            cache_path.display()
        );
        let bytes = std::fs::read(&cache_path).unwrap_or_else(|e| {
            panic!(
                "PR-2: failed to read {} for component `{name}`: {e}",
                cache_path.display()
            )
        });
        assert!(
            !bytes.is_empty(),
            "PR-2: cache/surfaces.yaml for `{name}` must not be empty"
        );
        let parsed: SurfacesFile = serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
            panic!("PR-2: failed to parse cache/surfaces.yaml for `{name}` as SurfacesFile: {e}")
        });
        assert_eq!(
            parsed.schema_version, SURFACES_SCHEMA_VERSION,
            "PR-2: cache/surfaces.yaml schema_version must match SURFACES_SCHEMA_VERSION"
        );
    }

    // AC(b): no surfaces.yaml must exist outside cache/ anywhere under
    // the fixture root. Walk the entire tmp tree.
    let violators = find_surfaces_yaml_outside_cache(tmp.path());
    assert!(
        violators.is_empty(),
        "PR-2: found surfaces.yaml file(s) outside cache/ sub-directory — old path must not exist:\n  {}",
        violators
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// AC(c): fingerprint-equality on re-run (cache-hit invariant).
// ---------------------------------------------------------------------------

#[test]
fn cache_surfaces_yaml_is_byte_identical_on_no_op_rerun() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    // First run — cold.
    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    let first_mylib = std::fs::read(tmp.path().join("mylib/.atlas/cache/surfaces.yaml"))
        .expect("PR-2: cache/surfaces.yaml must exist for mylib after first run");
    let first_mycli = std::fs::read(tmp.path().join("mycli/.atlas/cache/surfaces.yaml"))
        .expect("PR-2: cache/surfaces.yaml must exist for mycli after first run");

    // Second run — same inputs, same fingerprint. The pipeline's
    // unconditional write with atomic_write should produce identical bytes
    // (fingerprint-equality, not mtime).
    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    let second_mylib = std::fs::read(tmp.path().join("mylib/.atlas/cache/surfaces.yaml"))
        .expect("PR-2: cache/surfaces.yaml must exist for mylib after second run");
    let second_mycli = std::fs::read(tmp.path().join("mycli/.atlas/cache/surfaces.yaml"))
        .expect("PR-2: cache/surfaces.yaml must exist for mycli after second run");

    assert_eq!(
        first_mylib, second_mylib,
        "PR-2: mylib cache/surfaces.yaml must be byte-identical across two consecutive \
         runs with identical inputs"
    );
    assert_eq!(
        first_mycli, second_mycli,
        "PR-2: mycli cache/surfaces.yaml must be byte-identical across two consecutive \
         runs with identical inputs"
    );
}

// ---------------------------------------------------------------------------
// AC(d): .atlas/.gitignore exists at each component scope after run.
// ---------------------------------------------------------------------------

#[test]
fn atlas_gitignore_exists_at_each_component_scope() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    for name in ["mylib", "mycli"] {
        let gitignore_path = tmp.path().join(name).join(".atlas/.gitignore");
        assert!(
            gitignore_path.exists(),
            "PR-2: expected .atlas/.gitignore at component `{name}` scope: {} — does not exist",
            gitignore_path.display()
        );
        let content = std::fs::read_to_string(&gitignore_path)
            .unwrap_or_else(|e| panic!("PR-2: failed to read .atlas/.gitignore for `{name}`: {e}"));
        assert!(
            content.contains("cache/"),
            "PR-2: .atlas/.gitignore for `{name}` must contain `cache/`; got:\n{content}"
        );
    }
}
