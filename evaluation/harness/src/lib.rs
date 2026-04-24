//! Evaluation harness for Atlas.
//!
//! Three concerns, kept in separate modules:
//!
//! - `invariants`: structural checks that hold for any well-formed
//!   Atlas output (path coverage, edge participant existence, ...).
//!   These run regardless of whether a golden exists.
//! - `diff`: metric computation (component coverage, kind accuracy,
//!   edge precision/recall, identifier stability) comparing a tool run
//!   to a hand-authored golden, and a differential between two runs.
//! - `report`: persists per-run results as YAML and renders an HTML
//!   trend page from all YAMLs under `evaluation/results/`.
//!
//! `runner` ties the three together: load a tool output directory, run
//! every available check, produce a `RunReport`.
//!
//! The crate deliberately keeps its dep surface small (no Salsa, no
//! atlas-engine, no atlas-llm) so evaluation can be rebuilt and run
//! independently of the discovery pipeline it measures.

pub mod diff;
pub mod invariants;
pub mod report;
pub mod runner;

pub use diff::{
    component_coverage, edge_precision_recall, identifier_stability, kind_accuracy, run_diff,
    DifferentialReport, MetricSummary, OverlapThreshold, DEFAULT_OVERLAP_THRESHOLD,
};
pub use invariants::{
    collect_manifests_conservative, edge_participant_existence, fixedpoint_termination,
    git_boundary_rationale, manifest_coverage, no_path_overlap, path_coverage,
    rename_round_trip_holds, InvariantFailure, InvariantOutcome, InvariantReport,
};
pub use report::{render_trend_html, write_result_yaml, ResultFile};
pub use runner::{run_against_golden, run_without_golden, RunInputs, RunReport};
