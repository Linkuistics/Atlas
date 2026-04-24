//! Runs every harness check against a tool-output directory and
//! produces a `RunReport`. Two paths:
//!
//! - `run_against_golden`: invariants + metrics. Caller supplies the
//!   directory containing `components.golden.yaml` and friends.
//! - `run_without_golden`: invariants only. Used for tiny fixtures and
//!   bring-up smoke tests where no golden exists yet.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::diff::{metric_summary, MetricSummary, OverlapThreshold};
use crate::invariants::{
    edge_participant_existence, fixedpoint_termination, git_boundary_rationale, manifest_coverage,
    no_path_overlap, path_coverage, InvariantOutcome, InvariantReport,
};
use crate::report::ResultFile;

#[derive(Debug)]
pub struct RunReport {
    pub invariants: InvariantReport,
    pub metrics: Option<MetricSummary>,
    pub target: String,
    pub generated_at: String,
    pub notes: Option<String>,
}

impl RunReport {
    pub fn into_result_file(self) -> ResultFile {
        ResultFile {
            generated_at: self.generated_at,
            target: self.target,
            metrics: self.metrics,
            invariants: self.invariants,
            notes: self.notes,
        }
    }
}

/// Inputs shared by both entry points. `iterations` is the fixedpoint
/// iteration count emitted by the tool (if known) — the harness does
/// not parse the tool's CLI summary itself; the caller passes the
/// number through. `target_label` becomes the `target` field of the
/// resulting YAML file; it's the identity of the evaluation run, not a
/// filesystem path.
pub struct RunInputs<'a> {
    pub tool_output_dir: &'a Path,
    pub target_root: &'a Path,
    pub target_label: &'a str,
    pub generated_at: &'a str,
    pub iterations: Option<u32>,
    pub overlap_threshold: OverlapThreshold,
}

pub fn run_without_golden(inputs: RunInputs<'_>) -> Result<RunReport> {
    let triple = load_triple(inputs.tool_output_dir)?;
    let invariants = check_invariants(&triple, inputs.target_root, inputs.iterations);
    Ok(RunReport {
        invariants,
        metrics: None,
        target: inputs.target_label.into(),
        generated_at: inputs.generated_at.into(),
        notes: Some("no golden; structural invariants only".into()),
    })
}

pub fn run_against_golden(inputs: RunInputs<'_>, golden_dir: &Path) -> Result<RunReport> {
    let tool = load_triple(inputs.tool_output_dir)?;
    let golden = load_golden_triple(golden_dir)?;
    let invariants = check_invariants(&tool, inputs.target_root, inputs.iterations);
    let metrics = metric_summary(
        &golden.components,
        &tool.components,
        &golden.related,
        &tool.related,
        inputs.overlap_threshold,
    );
    Ok(RunReport {
        invariants,
        metrics: Some(metrics),
        target: inputs.target_label.into(),
        generated_at: inputs.generated_at.into(),
        notes: None,
    })
}

struct LoadedTriple {
    components: atlas_index::ComponentsFile,
    externals: atlas_index::ExternalsFile,
    related: atlas_index::RelatedComponentsFile,
}

fn load_triple(dir: &Path) -> Result<LoadedTriple> {
    Ok(LoadedTriple {
        components: atlas_index::load_or_default_components(&dir.join("components.yaml"))
            .with_context(|| format!("load components.yaml from {}", dir.display()))?,
        externals: atlas_index::load_or_default_externals(
            &dir.join("external-components.yaml"),
        )
        .with_context(|| format!("load external-components.yaml from {}", dir.display()))?,
        related: component_ontology::load_or_default(&dir.join("related-components.yaml"))
            .with_context(|| format!("load related-components.yaml from {}", dir.display()))?,
    })
}

fn load_golden_triple(dir: &Path) -> Result<LoadedTriple> {
    // Goldens live under different filenames — `*.golden.yaml`. Fall
    // back to the regular names if goldens aren't there, so callers can
    // point the golden_dir at another run's output to do a regression
    // comparison.
    let components_path = first_existing(
        dir,
        &["components.golden.yaml", "components.yaml"],
    );
    let externals_path = first_existing(
        dir,
        &["external-components.golden.yaml", "external-components.yaml"],
    );
    let related_path = first_existing(
        dir,
        &["related-components.golden.yaml", "related-components.yaml"],
    );
    Ok(LoadedTriple {
        components: atlas_index::load_or_default_components(&components_path)
            .with_context(|| format!("load golden components from {}", dir.display()))?,
        externals: atlas_index::load_or_default_externals(&externals_path)
            .with_context(|| format!("load golden externals from {}", dir.display()))?,
        related: component_ontology::load_or_default(&related_path)
            .with_context(|| format!("load golden related from {}", dir.display()))?,
    })
}

fn first_existing(dir: &Path, names: &[&str]) -> PathBuf {
    for name in names {
        let p = dir.join(name);
        if p.exists() {
            return p;
        }
    }
    // None exist — return the first (will trigger schema-default load).
    dir.join(names[0])
}

fn check_invariants(
    triple: &LoadedTriple,
    target_root: &Path,
    iterations: Option<u32>,
) -> InvariantReport {
    let mut report = InvariantReport::default();

    report.record(
        "path_coverage",
        outcome(path_coverage(&triple.components)),
    );
    report.record(
        "no_path_overlap",
        outcome(no_path_overlap(&triple.components)),
    );
    if target_root.exists() {
        report.record(
            "manifest_coverage",
            outcome(manifest_coverage(&triple.components, target_root)),
        );
        report.record(
            "git_boundary_rationale",
            outcome(git_boundary_rationale(&triple.components, target_root)),
        );
    } else {
        report.record(
            "manifest_coverage",
            InvariantOutcome::Skipped {
                reason: format!("target_root {} does not exist", target_root.display()),
            },
        );
        report.record(
            "git_boundary_rationale",
            InvariantOutcome::Skipped {
                reason: format!("target_root {} does not exist", target_root.display()),
            },
        );
    }
    report.record(
        "edge_participant_existence",
        outcome(edge_participant_existence(
            &triple.components,
            &triple.externals,
            &triple.related,
        )),
    );
    report.record(
        "fixedpoint_termination",
        match fixedpoint_termination(iterations, 8) {
            Ok(()) if iterations.is_none() => InvariantOutcome::Skipped {
                reason: "tool did not emit iteration count".into(),
            },
            other => outcome(other),
        },
    );
    // rename_round_trip needs a pair of runs; left to the differential
    // evaluation path. Record as skipped so the report always has a
    // consistent row set.
    report.record(
        "rename_round_trip",
        InvariantOutcome::Skipped {
            reason: "requires a prior run; use differential evaluation".into(),
        },
    );
    report
}

fn outcome<E: std::fmt::Display>(r: Result<(), E>) -> InvariantOutcome {
    match r {
        Ok(()) => InvariantOutcome::Pass,
        Err(e) => InvariantOutcome::Fail {
            message: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use atlas_index::{
        CacheFingerprints, ComponentEntry, ComponentsFile, PathSegment,
        COMPONENTS_SCHEMA_VERSION,
    };
    use component_ontology::{EvidenceGrade, LifecycleScope, RelatedComponentsFile};
    use tempfile::TempDir;

    use super::*;

    fn component(id: &str, path: &str) -> ComponentEntry {
        ComponentEntry {
            id: id.into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![LifecycleScope::Build],
            language: Some("rust".into()),
            build_system: Some("cargo".into()),
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from(path),
                content_sha: format!("sha-{path}"),
            }],
            manifests: vec![PathBuf::from(format!("{path}/Cargo.toml"))],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["manifest".into()],
            rationale: "r".into(),
            deleted: false,
        }
    }

    fn write_tool_output(dir: &Path, components: &ComponentsFile) {
        atlas_index::save_components_atomic(&dir.join("components.yaml"), components).unwrap();
        atlas_index::save_externals_atomic(
            &dir.join("external-components.yaml"),
            &atlas_index::ExternalsFile::default(),
        )
        .unwrap();
        component_ontology::save_atomic(
            &dir.join("related-components.yaml"),
            &RelatedComponentsFile::default(),
        )
        .unwrap();
    }

    #[test]
    fn run_without_golden_records_structural_outcomes_only() {
        let tool_dir = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();
        std::fs::create_dir(target.path().join("pkg")).unwrap();
        std::fs::write(target.path().join("pkg/Cargo.toml"), "[package]\n").unwrap();

        let mut c = component("pkg", "pkg");
        c.manifests = vec![PathBuf::from("pkg/Cargo.toml")];
        let components = ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            root: target.path().to_path_buf(),
            generated_at: "".into(),
            cache_fingerprints: CacheFingerprints::default(),
            components: vec![c],
        };
        write_tool_output(tool_dir.path(), &components);

        let report = run_without_golden(RunInputs {
            tool_output_dir: tool_dir.path(),
            target_root: target.path(),
            target_label: "tiny",
            generated_at: "2026-04-24",
            iterations: Some(2),
            overlap_threshold: OverlapThreshold::default(),
        })
        .unwrap();

        assert!(report.metrics.is_none());
        assert!(
            report.invariants.all_passed(),
            "invariants failed: {:?}",
            report.invariants.failures().collect::<Vec<_>>()
        );
    }

    #[test]
    fn run_against_golden_computes_metrics() {
        let tool_dir = TempDir::new().unwrap();
        let golden_dir = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        std::fs::create_dir(target.path().join("pkg")).unwrap();
        std::fs::write(target.path().join("pkg/Cargo.toml"), "[package]\n").unwrap();

        let mut c = component("pkg", "pkg");
        c.manifests = vec![PathBuf::from("pkg/Cargo.toml")];
        let components_file = ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            root: target.path().to_path_buf(),
            generated_at: "".into(),
            cache_fingerprints: CacheFingerprints::default(),
            components: vec![c],
        };
        write_tool_output(tool_dir.path(), &components_file);

        // Golden file under *.golden.yaml name.
        atlas_index::save_components_atomic(
            &golden_dir.path().join("components.golden.yaml"),
            &components_file,
        )
        .unwrap();

        let report = run_against_golden(
            RunInputs {
                tool_output_dir: tool_dir.path(),
                target_root: target.path(),
                target_label: "tiny",
                generated_at: "2026-04-24",
                iterations: None,
                overlap_threshold: OverlapThreshold::default(),
            },
            golden_dir.path(),
        )
        .unwrap();

        let metrics = report.metrics.expect("metrics expected when golden present");
        assert!((metrics.component_coverage - 1.0).abs() < 1e-6);
        assert!((metrics.spurious_rate - 0.0).abs() < 1e-6);
    }

    #[test]
    fn fixedpoint_skipped_when_iterations_none() {
        let tool_dir = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();
        write_tool_output(tool_dir.path(), &ComponentsFile::default());

        let report = run_without_golden(RunInputs {
            tool_output_dir: tool_dir.path(),
            target_root: target.path(),
            target_label: "empty",
            generated_at: "2026-04-24",
            iterations: None,
            overlap_threshold: OverlapThreshold::default(),
        })
        .unwrap();

        assert!(matches!(
            report.invariants.outcomes.get("fixedpoint_termination"),
            Some(InvariantOutcome::Skipped { .. })
        ));
    }
}
