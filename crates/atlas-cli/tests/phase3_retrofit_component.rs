//! Phase 3 PR-3 sweep test — per-component `cache/component.yaml` retrofit.
//!
//! Acceptance criteria (plan §4 PR-3, §5 row):
//!
//! (a) After `run_index` completes, every live component has
//!     `<component>/.atlas/cache/component.yaml` populated with
//!     non-empty `analyser_id` and `analyser_version` fields
//!     (Phase 2 PR-4 invariant).
//! (b) No per-component `component.yaml` file exists in a component's
//!     `.atlas/` directory unless it is under the `cache/` sub-path.
//!     I.e. the old non-cache location is empty after the retrofit.
//!
//! The test uses the `tiny` fixture (two-crate Rust workspace).
//!
//! Fixture-build boilerplate (`materialise_fixture`, `base_config`,
//! `run_with`, the canned-response backend) lives in the shared
//! `tests/common/sweep_support.rs` module — see Phase 4 PR-6.

use std::path::{Path, PathBuf};

use atlas_index::{load_or_default_components, PerComponentFile};

mod common;
use common::sweep_support::*;

// ---------------------------------------------------------------------------
// AC(a): every live component has cache/component.yaml with analyser fields.
// ---------------------------------------------------------------------------

/// PR-3 acceptance: after `run_index`, every live component must have a
/// `cache/component.yaml` inside its `.atlas/` directory, and that file
/// must carry non-empty `analyser_id` and `analyser_version` fields
/// (PR-4 invariant preserved through the PR-3 path retrofit).
#[test]
fn every_live_component_has_cache_component_yaml_with_analyser_fields() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    // Load the top-level components list (now at cache/components.yaml).
    let components_path = config.output_dir.join("cache/components.yaml");
    let components = load_or_default_components(&components_path)
        .expect("load_or_default_components must succeed");

    let live: Vec<_> = components
        .components
        .iter()
        .filter(|c| !c.deleted)
        .collect();

    assert!(
        !live.is_empty(),
        "PR-3 sweep: tiny fixture must produce at least one live component"
    );

    for entry in &live {
        let segment = entry
            .path_segments
            .first()
            .unwrap_or_else(|| panic!("component `{}` has no path_segments", entry.id.as_str()));
        let component_dir = tmp.path().join(&segment.path);

        // AC(a): the cache/component.yaml must exist.
        let cache_path = component_dir
            .join(".atlas")
            .join("cache")
            .join("component.yaml");
        assert!(
            cache_path.exists(),
            "PR-3 sweep: expected cache/component.yaml at {} for component `{}`",
            cache_path.display(),
            entry.id.as_str()
        );

        // Parse and validate the analyser identity fields (PR-4 invariant).
        let bytes = std::fs::read(&cache_path).unwrap_or_else(|e| {
            panic!(
                "failed to read cache/component.yaml at {}: {e}",
                cache_path.display()
            )
        });
        let parsed: PerComponentFile = serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "failed to parse {} as PerComponentFile: {e}",
                cache_path.display()
            )
        });

        assert!(
            !parsed.analyser_id.is_empty(),
            "PR-3 sweep: component `{}` cache/component.yaml must have non-empty analyser_id",
            entry.id.as_str()
        );
        assert!(
            !parsed.analyser_version.is_empty(),
            "PR-3 sweep: component `{}` cache/component.yaml must have non-empty analyser_version",
            entry.id.as_str()
        );
    }
}

// ---------------------------------------------------------------------------
// AC(b): no per-component component.yaml exists outside the cache/ path.
// ---------------------------------------------------------------------------

/// PR-3 acceptance: after `run_index`, no component's `.atlas/` directory
/// contains a `component.yaml` file directly (i.e. not under `cache/`).
/// The old non-cache location must be absent.
#[test]
fn no_per_component_yaml_outside_cache() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new(sweep_fingerprint()));

    // Load the live component list to know which component directories to check.
    let components_path = config.output_dir.join("cache/components.yaml");
    let components = load_or_default_components(&components_path)
        .expect("load_or_default_components must succeed");

    let live: Vec<_> = components
        .components
        .iter()
        .filter(|c| !c.deleted)
        .collect();

    let mut violators: Vec<String> = Vec::new();

    for entry in &live {
        let segment = entry
            .path_segments
            .first()
            .unwrap_or_else(|| panic!("component `{}` has no path_segments", entry.id.as_str()));
        let component_dir = tmp.path().join(&segment.path);

        // The old path — directly in .atlas/ without cache/ — must not exist.
        let old_path = component_dir.join(".atlas").join("component.yaml");
        if old_path.exists() {
            violators.push(format!(
                "component `{}`: found stale component.yaml at {}",
                entry.id.as_str(),
                old_path.display()
            ));
        }
    }

    // Also walk the entire tmp tree for any stray component.yaml files that
    // are NOT under a cache/ directory, to catch regressions from future
    // code changes that accidentally write to the old location.
    let stray = find_component_yaml_outside_cache(tmp.path());
    for path in stray {
        let s = path.display().to_string();
        if !violators.iter().any(|v| v.contains(&s)) {
            violators.push(format!("stray component.yaml outside cache/ at {s}"));
        }
    }

    assert!(
        violators.is_empty(),
        "PR-3 sweep: found component.yaml file(s) outside cache/ sub-directory:\n  {}",
        violators.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Helper: walk the tree and find component.yaml files not under cache/.
// ---------------------------------------------------------------------------

/// Walk `root` recursively and collect all `component.yaml` paths that
/// are NOT inside a `cache/` path component (and not named `components.yaml`).
fn find_component_yaml_outside_cache(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk_for_component_yaml(root, &mut found);
    found
}

fn walk_for_component_yaml(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_for_component_yaml(&path, out);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Match singular `component.yaml` only (not plural `components.yaml`).
            if name == "component.yaml" {
                // The path is acceptable only if one of its ancestors is `cache`.
                let under_cache = path
                    .components()
                    .any(|c| c.as_os_str() == std::ffi::OsStr::new("cache"));
                if !under_cache {
                    out.push(path);
                }
            }
        }
    }
}
