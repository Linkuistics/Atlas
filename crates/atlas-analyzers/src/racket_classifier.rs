//! Racket L3 classifier (Atlas vNext Phase 2 PR-9).
//!
//! Sibling of [`crate::python_classifier`] and
//! [`crate::cargo_classifier`]: an L3 deterministic analyser that
//! emits `kind: racket-package` whenever a candidate dir carries an
//! `info.rkt` manifest (Racket's canonical package-manifest file).
//!
//! A directory with `*.rkt` files but no `info.rkt` declines — the
//! `info.rkt` requirement follows §4 PR-9 which specifies:
//! "L3 classifier — `info.rkt` → `racket-package`. `*.rkt` without
//! `info.rkt` → declines."
//!
//! ## Why this is a separate analyser
//!
//! Mirrors the Python classifier pattern: a first-class registry
//! analyser keeps the registry sha in sync with the vocabulary and
//! propagates the analyser-id to per-component YAML for PR-4's
//! plumbing.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id. Matches the wire form a future
/// `analyzers.yaml` would carry.
pub const ANALYZER_ID: &str = "racket-classifier";

/// Bumped when the rule table changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output shape mirroring [`crate::python_classifier::PythonClassificationOutput`].
/// The L3 adapter downcasts the `Box<dyn StageOutput>` to this struct
/// and translates onto `Classification`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RacketClassificationOutput {
    /// Always `"racket-package"` for Phase 2 PR-9.
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
pub struct RacketClassifier;

impl RacketClassifier {
    pub fn new() -> Self {
        RacketClassifier
    }
}

impl Analyzer for RacketClassifier {
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
        // Only applies when `info.rkt` is present. A directory with
        // `*.rkt` files but no `info.rkt` declines (§4 PR-9).
        target.manifest_by_name("info.rkt").is_some()
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        // Contribute the `info.rkt` content sha so a manifest change
        // reshapes the L3 cache key for this analyser.
        if let Some(m) = target.manifest_by_name("info.rkt") {
            vec![FingerprintInput::FileContentSha(m.content_sha.clone())]
        } else {
            Vec::new()
        }
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        if target.manifest_by_name("info.rkt").is_none() {
            return AnalyzerResult::Declines;
        }
        AnalyzerResult::Confident(Box::new(RacketClassificationOutput {
            kind: "racket-package".into(),
            lifecycle_roles: vec!["build".into(), "runtime".into()],
            build_system: Some("raco".into()),
            role: None,
            language: "racket".into(),
            evidence_fields: vec!["info.rkt".into()],
            rationale: "info.rkt present — Racket package manifest.".into(),
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
    fn info_rkt_yields_racket_package() {
        let target = target_with_manifests(&[("info.rkt", "#lang info\n(define name \"x\")\n")]);
        let an = RacketClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let rkt = out
                    .as_any()
                    .downcast_ref::<RacketClassificationOutput>()
                    .expect("output is RacketClassificationOutput");
                assert_eq!(rkt.kind, "racket-package");
                assert_eq!(rkt.language, "racket");
                assert_eq!(rkt.build_system.as_deref(), Some("raco"));
                assert!(rkt.is_boundary);
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn applies_is_false_without_info_rkt() {
        let target = target_with_manifests(&[("Cargo.toml", "[package]\n")]);
        assert!(!RacketClassifier::new().applies(&target));
    }

    #[test]
    fn applies_is_false_for_rkt_file_without_info_rkt() {
        // §4 PR-9: `*.rkt` without `info.rkt` → declines.
        let target = target_with_manifests(&[("main.rkt", "#lang racket\n")]);
        assert!(!RacketClassifier::new().applies(&target));
    }

    #[test]
    fn fingerprint_inputs_returns_info_rkt_content_sha() {
        let target = target_with_manifests(&[("info.rkt", "#lang info\n")]);
        let inputs = RacketClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 1);
        assert!(matches!(inputs[0], FingerprintInput::FileContentSha(_)));
    }

    #[test]
    fn fingerprint_inputs_empty_without_info_rkt() {
        let target = target_with_manifests(&[]);
        let inputs = RacketClassifier::new().fingerprint_inputs(&target);
        assert!(inputs.is_empty());
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = RacketClassifier::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L3);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }
}
