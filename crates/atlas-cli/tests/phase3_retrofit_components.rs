//! Phase 3 PR-4 retrofit sweep test.
//!
//! Acceptance criteria verified here (plan §4 PR-4 / §5 gate):
//!
//! 1. After `atlas index` the top-level `components.yaml` is written at
//!    `<root>/.atlas/cache/components.yaml`. The old top-level location
//!    (directly inside `.atlas/`, without the `cache/` sub-directory)
//!    must be absent.
//! 2. No `components.yaml` file exists in `.atlas/` outside the `cache/`
//!    sub-directory.
//! 3. The `cache/components.yaml` contains the expected components
//!    (parsed as `ComponentsFile`).
//! 4. Bit-for-bit stability: two consecutive runs with identical inputs
//!    produce byte-identical `cache/components.yaml` output.
//!
//! Fixture-build boilerplate (`materialise_fixture`, `base_config`,
//! `run_with`, `LenientBackend`) lives in the shared
//! `tests/common/sweep_support.rs` module — see Phase 4 PR-6.

use std::path::{Path, PathBuf};

use atlas_index::{load_or_default_components, ComponentsFile};

mod common;
use common::sweep_support::*;

// ---------------------------------------------------------------------------
// AC#1 + AC#2: cache/components.yaml populated, no top-level components.yaml
// ---------------------------------------------------------------------------

#[test]
fn cache_components_yaml_written_and_no_top_level_file() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    // AC#1: components.yaml must exist at the new cache location.
    let cache_path = config.output_dir.join("cache/components.yaml");
    assert!(
        cache_path.exists(),
        "PR-4: cache/components.yaml must be written by atlas index; \
         expected at {}",
        cache_path.display()
    );

    // AC#2: NO components.yaml must exist directly in .atlas/ (the old path).
    let old_path = config.output_dir.join("components.yaml");
    assert!(
        !old_path.exists(),
        "PR-4: components.yaml must NOT exist at the old top-level path {}; \
         the file must only live in cache/",
        old_path.display()
    );
}

// ---------------------------------------------------------------------------
// AC#3: cache/components.yaml parses as ComponentsFile and contains
// expected components from the tiny fixture.
// ---------------------------------------------------------------------------

#[test]
fn cache_components_yaml_contains_expected_components() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    let cache_path = config.output_dir.join("cache/components.yaml");
    let bytes = std::fs::read(&cache_path).unwrap_or_else(|e| {
        panic!(
            "failed to read cache/components.yaml at {}: {e}",
            cache_path.display()
        )
    });

    let parsed: ComponentsFile = serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "failed to parse {} as ComponentsFile: {e}",
            cache_path.display()
        )
    });

    // The tiny fixture has a library (mylib) and a CLI (mycli); both
    // must appear as live (non-deleted) components.
    let live: Vec<_> = parsed.components.iter().filter(|c| !c.deleted).collect();
    assert!(
        live.len() >= 2,
        "cache/components.yaml must contain at least 2 live components \
         (mylib + mycli); got {} live out of {}: {:?}",
        live.len(),
        parsed.components.len(),
        live.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
    );

    // Verify the `load_or_default_components` API also finds the file
    // at the cache path (it reads from the path we pass, so the key
    // assertion is that the path itself is correct).
    let via_load =
        load_or_default_components(&cache_path).expect("load_or_default_components succeeds");
    assert_eq!(
        via_load.components.len(),
        parsed.components.len(),
        "load_or_default_components must read the same file as direct read"
    );
}

// ---------------------------------------------------------------------------
// AC#4: bit-for-bit stability across two consecutive runs.
// ---------------------------------------------------------------------------

#[test]
fn cache_components_yaml_is_byte_identical_on_no_op_rerun() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    // First run — cold.
    run_with(&config, LenientBackend::new(sweep_fingerprint()));
    let first = std::fs::read(config.output_dir.join("cache/components.yaml"))
        .expect("cache/components.yaml must exist after first run");

    // Second run — same inputs, same fingerprint. The pipeline's
    // `stable_generated_at` heuristic should preserve the timestamp,
    // making the file byte-identical.
    run_with(&config, LenientBackend::new(sweep_fingerprint()));
    let second = std::fs::read(config.output_dir.join("cache/components.yaml"))
        .expect("cache/components.yaml must exist after second run");

    assert_eq!(
        first, second,
        "cache/components.yaml must be byte-identical across two consecutive \
         runs with identical inputs (PR-4 bit-for-bit stability criterion)"
    );
}

// ---------------------------------------------------------------------------
// AC#5 (sweep): walk the entire .atlas/ directory tree and assert that
// no file named `components.yaml` lives anywhere except under `cache/`.
// ---------------------------------------------------------------------------

#[test]
fn no_components_yaml_outside_cache_in_atlas_dir() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    // Walk every file under .atlas/ and collect any `components.yaml`
    // that does NOT live in a `cache/` sub-path.
    let atlas_dir = &config.output_dir;
    let violators: Vec<PathBuf> = find_components_yaml_outside_cache(atlas_dir);

    assert!(
        violators.is_empty(),
        "PR-4 sweep: found components.yaml file(s) outside cache/ sub-directory:\n  {}",
        violators
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// Recursively walk `dir` and collect all `components.yaml` paths that
/// are NOT inside a `cache/` component of their path. The name match
/// uses the file's basename; the cache check uses the path components.
fn find_components_yaml_outside_cache(dir: &Path) -> Vec<PathBuf> {
    let mut violators = Vec::new();
    walk_dir(dir, &mut violators);
    violators
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, out);
        } else {
            let is_components_yaml = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "components.yaml")
                .unwrap_or(false);
            if is_components_yaml {
                // A path is "inside cache/" when one of its ancestors is
                // named `cache`. We check by scanning the path components.
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
