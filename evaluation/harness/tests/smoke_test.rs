//! End-to-end smoke test: construct a plausible Atlas output for the
//! existing tiny fixture (`crates/atlas-cli/tests/fixtures/tiny`), run
//! the harness against it, and assert the result YAML has every field
//! the trend page expects.
//!
//! This test does NOT invoke the real Atlas binary — that would burn
//! LLM tokens on every `cargo test`. A separate gated test (behind
//! `ATLAS_LLM_RUN_CLAUDE_TESTS=1`) could run the real pipeline; not
//! included here because the exit criterion is about harness shape,
//! not tool correctness.

use std::path::{Path, PathBuf};

use atlas_eval::{
    render_trend_html, run_without_golden, write_result_yaml, OverlapThreshold, RunInputs,
};
use atlas_index::{
    CacheFingerprints, ComponentEntry, ComponentsFile, ExternalsFile, PathSegment,
    COMPONENTS_SCHEMA_VERSION, EXTERNALS_SCHEMA_VERSION,
};
use component_ontology::{EvidenceGrade, LifecycleScope, RelatedComponentsFile};
use tempfile::TempDir;

fn tiny_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/atlas-cli/tests/fixtures/tiny")
}

fn plausible_tiny_output(
    target_root: &Path,
) -> (ComponentsFile, ExternalsFile, RelatedComponentsFile) {
    // The tiny fixture has two crates: mycli and mylib.
    let components = ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: target_root.to_path_buf(),
        generated_at: "2026-04-24T00:00:00Z".into(),
        cache_fingerprints: CacheFingerprints::default(),
        components: vec![
            ComponentEntry {
                id: "mycli".into(),
                parent: None,
                kind: "rust-cli".into(),
                lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
                language: Some("rust".into()),
                build_system: Some("cargo".into()),
                role: Some("cli".into()),
                path_segments: vec![PathSegment {
                    path: PathBuf::from("mycli"),
                    content_sha: "fakesha-mycli".into(),
                }],
                manifests: vec![PathBuf::from("mycli/Cargo.toml")],
                doc_anchors: vec![],
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec!["Cargo.toml:[[bin]]".into()],
                rationale: "Cargo.toml declares a binary target".into(),
                deleted: false,
            },
            ComponentEntry {
                id: "mylib".into(),
                parent: None,
                kind: "rust-library".into(),
                lifecycle_roles: vec![LifecycleScope::Build],
                language: Some("rust".into()),
                build_system: Some("cargo".into()),
                role: Some("library".into()),
                path_segments: vec![PathSegment {
                    path: PathBuf::from("mylib"),
                    content_sha: "fakesha-mylib".into(),
                }],
                manifests: vec![PathBuf::from("mylib/Cargo.toml")],
                doc_anchors: vec![],
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec!["Cargo.toml:[lib]".into()],
                rationale: "Cargo.toml declares a library target".into(),
                deleted: false,
            },
        ],
    };
    let externals = ExternalsFile {
        schema_version: EXTERNALS_SCHEMA_VERSION,
        externals: vec![],
    };
    let related = RelatedComponentsFile::default();
    (components, externals, related)
}

fn write_tool_triple(dir: &Path, c: &ComponentsFile, e: &ExternalsFile, r: &RelatedComponentsFile) {
    atlas_index::save_components_atomic(&dir.join("components.yaml"), c).unwrap();
    atlas_index::save_externals_atomic(&dir.join("external-components.yaml"), e).unwrap();
    component_ontology::save_atomic(&dir.join("related-components.yaml"), r).unwrap();
}

#[test]
fn harness_runs_against_tiny_fixture_and_emits_results_yaml() {
    let target_root = tiny_fixture_dir();
    assert!(
        target_root.exists(),
        "tiny fixture missing at {}",
        target_root.display()
    );

    let tool_output = TempDir::new().unwrap();
    let (c, e, r) = plausible_tiny_output(&target_root);
    write_tool_triple(tool_output.path(), &c, &e, &r);

    let report = run_without_golden(RunInputs {
        tool_output_dir: tool_output.path(),
        target_root: &target_root,
        target_label: "tiny",
        generated_at: "2026-04-24",
        iterations: Some(1),
        overlap_threshold: OverlapThreshold::default(),
    })
    .expect("harness runs without error");

    assert!(
        report.invariants.all_passed(),
        "tiny-fixture invariants should pass: {:?}",
        report
            .invariants
            .failures()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        report.metrics.is_none(),
        "no golden → metrics should be absent"
    );

    let results_dir = TempDir::new().unwrap();
    let result_file_path = results_dir.path().join("2026-04-24-tiny.yaml");
    write_result_yaml(&result_file_path, &report.into_result_file()).unwrap();

    let yaml = std::fs::read_to_string(&result_file_path).unwrap();
    assert!(yaml.contains("target: tiny"));
    assert!(yaml.contains("2026-04-24"), "generated_at missing: {yaml}");
    assert!(yaml.contains("invariants:"));
    // The comment in the task states metrics may be absent on no-golden
    // runs. Make that contract explicit.
    assert!(
        yaml.contains("metrics: null") || !yaml.contains("component_coverage"),
        "metrics should be absent on no-golden runs: {yaml}"
    );

    // Trend page renders.
    let trend_html = results_dir.path().join("trend.html");
    render_trend_html(results_dir.path(), &trend_html).unwrap();
    let html = std::fs::read_to_string(&trend_html).unwrap();
    assert!(html.contains("Atlas evaluation trend"));
    assert!(html.contains("tiny"));
}
