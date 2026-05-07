//! Elixir L3 classifier (Atlas vNext Phase 2 PR-8).
//!
//! Sibling of [`crate::python_classifier`] and
//! [`crate::cargo_classifier`]: an L3 deterministic analyser that
//! emits `kind: elixir-project` whenever a candidate dir carries
//! a `mix.exs` manifest.
//!
//! ## Why this is a separate analyser
//!
//! The plan §4 PR-8 brief calls out an Elixir *L3 classifier* as a
//! sibling of the new `elixir-project` kind. Lifting the rule into a
//! standalone analyser closes the loop: the registry's deterministic
//! pass owns the verdict, the analyser-registry sha contributes to L3
//! cache invalidation, and the analyser-id propagates to per-component
//! YAML via PR-4's plumbing.
//!
//! ## Phase 2 evidence
//!
//! `evidence_grade: Strong` — manifest presence is unambiguous.
//! `evidence_fields` lists the manifest basename so the per-component
//! rationale is auditable.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id. Matches the wire form a future `analyzers.yaml`
/// would carry.
pub const ANALYZER_ID: &str = "elixir-classifier";

/// Bumped when the rule changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output shape mirroring [`crate::python_classifier::PythonClassificationOutput`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElixirClassificationOutput {
    /// Always `"elixir-project"` for Phase 2 PR-8.
    pub kind: String,
    pub lifecycle_roles: Vec<String>,
    pub build_system: Option<String>,
    pub role: Option<String>,
    pub language: String,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
    pub is_boundary: bool,
}

/// The classifier itself. Stateless.
#[derive(Debug, Default)]
pub struct ElixirClassifier;

impl ElixirClassifier {
    pub fn new() -> Self {
        ElixirClassifier
    }
}

impl Analyzer for ElixirClassifier {
    fn id(&self) -> &str {
        ANALYZER_ID
    }
    fn stage(&self) -> Stage {
        Stage::L3
    }
    fn cost_class(&self) -> CostClass {
        CostClass::DeterministicCheap
    }
    fn version(&self) -> &str {
        ANALYZER_VERSION
    }

    fn applies(&self, target: &Target) -> bool {
        target.manifest_by_name("mix.exs").is_some()
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        let mut inputs = Vec::new();
        if let Some(m) = target.manifest_by_name("mix.exs") {
            inputs.push(FingerprintInput::FileContentSha(m.content_sha.clone()));
        }
        inputs
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        if target.manifest_by_name("mix.exs").is_none() {
            return AnalyzerResult::Declines;
        }
        AnalyzerResult::Confident(Box::new(ElixirClassificationOutput {
            kind: "elixir-project".into(),
            lifecycle_roles: vec!["build".into(), "runtime".into()],
            build_system: Some("mix".into()),
            role: None,
            language: "elixir".into(),
            evidence_fields: vec!["mix.exs".into()],
            rationale: "mix.exs present — Elixir Mix project.".into(),
            is_boundary: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TargetFile;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn target_with_manifests(files: &[(&str, &str)]) -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: files
                .iter()
                .map(|(name, body)| TargetFile {
                    name: (*name).into(),
                    relpath: PathBuf::from(name),
                    bytes: body.as_bytes().to_vec(),
                    content_sha: format!("sha-{}-{}", name, body.len()),
                })
                .collect(),
            top_level_files: files.iter().map(|(n, _)| (*n).to_string()).collect(),
        }
    }

    #[test]
    fn mix_exs_yields_elixir_project() {
        let target = target_with_manifests(&[("mix.exs", "defmodule MyApp.MixProject do\nend\n")]);
        let an = ElixirClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let el = out
                    .as_any()
                    .downcast_ref::<ElixirClassificationOutput>()
                    .expect("output is ElixirClassificationOutput");
                assert_eq!(el.kind, "elixir-project");
                assert_eq!(el.language, "elixir");
                assert_eq!(el.build_system.as_deref(), Some("mix"));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn applies_is_false_without_mix_exs() {
        let target = target_with_manifests(&[("Cargo.toml", "[package]\n")]);
        assert!(!ElixirClassifier::new().applies(&target));
    }

    #[test]
    fn no_mix_exs_declines() {
        let target = target_with_manifests(&[("Cargo.toml", "[package]\n")]);
        let an = ElixirClassifier::new();
        match an.analyse(&AnalysisContext::deterministic_only(), &target) {
            AnalyzerResult::Declines => {}
            other => panic!("expected Declines, got {other:?}"),
        }
    }

    #[test]
    fn fingerprint_inputs_one_per_present_manifest() {
        let target = target_with_manifests(&[("mix.exs", "x\n")]);
        let inputs = ElixirClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 1);
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = ElixirClassifier::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L3);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }
}
