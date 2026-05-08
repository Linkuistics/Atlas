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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_index::{load_or_default_components, ComponentsFile};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Minimal canned-response backend (mirrors pipeline_integration.rs
// `LenientBackend` — a single backend shape for the tiny fixture).
// ---------------------------------------------------------------------------

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0xCAu8; 32],
        ontology_sha: [0xCBu8; 32],
        model_id: "pr4-sweep-backend".into(),
        backend_version: "v-pr4-sweep".into(),
    }
}

struct LenientBackend {
    fingerprint: LlmFingerprint,
}

impl LenientBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            fingerprint: fingerprint(),
        })
    }
}

impl LlmBackend for LenientBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "sweep-backend lenient classify",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({"purpose": "stub", "notes": ""}),
            PromptId::Stage2Edges => json!([]),
            PromptId::Subcarve => json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "sweep-backend policy declined",
            }),
        })
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

// ---------------------------------------------------------------------------
// Fixture helpers — reuse the tiny fixture already in tests/fixtures/tiny/
// ---------------------------------------------------------------------------

fn tiny_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tiny")
}

fn copy_dir_all(src: &Path, dst: &Path) {
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

fn materialise_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    copy_dir_all(&tiny_fixture_root(), tmp.path());
    tmp
}

fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

fn run_with(config: &IndexConfig, backend: Arc<dyn LlmBackend>) {
    run_index(
        config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index must succeed");
}

// ---------------------------------------------------------------------------
// AC#1 + AC#2: cache/components.yaml populated, no top-level components.yaml
// ---------------------------------------------------------------------------

#[test]
fn cache_components_yaml_written_and_no_top_level_file() {
    let tmp = materialise_fixture();
    let config = base_config(tmp.path());

    run_with(&config, LenientBackend::new());

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

    run_with(&config, LenientBackend::new());

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
    run_with(&config, LenientBackend::new());
    let first = std::fs::read(config.output_dir.join("cache/components.yaml"))
        .expect("cache/components.yaml must exist after first run");

    // Second run — same inputs, same fingerprint. The pipeline's
    // `stable_generated_at` heuristic should preserve the timestamp,
    // making the file byte-identical.
    run_with(&config, LenientBackend::new());
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

    run_with(&config, LenientBackend::new());

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
