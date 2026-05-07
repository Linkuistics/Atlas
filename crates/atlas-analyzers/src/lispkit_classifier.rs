//! LispKit L3 classifier (Atlas vNext Phase 2 PR-10).
//!
//! Sibling of [`crate::python_classifier`]: an L3 deterministic
//! analyser that emits `kind: lispkit-package` whenever a candidate
//! dir contains one or more `*.sld` (Scheme Library Definition) files
//! — the R7RS-standard manifest convention for `define-library` forms.
//!
//! ## Manifest convention rationale
//!
//! `*.sld` is the unambiguous signal for a LispKit/R7RS component:
//!
//! - The R7RS standard itself uses `*.sld` in its examples.
//! - Every portable R7RS implementation (LispKit, Chibi-Scheme,
//!   Gauche, Chez) expects `*.sld` as the canonical library-declaration
//!   extension.
//! - Alternatives like `package.scm` or `lispkit.toml` are
//!   project-specific and non-standard.
//! - Without direct access to the Linkuistics project, `*.sld` is the
//!   safest fallback: every LispKit library ships at least one.
//!
//! The classifier probes `target.top_level_files` and
//! `target.manifests` for the `*.sld` extension. Because `*.sld` is
//! not in the engine's `EXACT_MANIFEST_BASENAMES` list (it is a glob
//! pattern rather than an exact name), the engine may not pre-load the
//! file; this classifier therefore also probes `top_level_files` for
//! any basename ending in `.sld`.
//!
//! ## Evidence
//!
//! `evidence_grade: Strong` — `*.sld` presence is unambiguous.
//! `evidence_fields` lists the matching filename.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id.
pub const ANALYZER_ID: &str = "lispkit-classifier";

/// Bumped when the rule table changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output shape mirroring [`crate::python_classifier::PythonClassificationOutput`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LispKitClassificationOutput {
    /// Always `"lispkit-package"` for Phase 2 PR-10.
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
pub struct LispKitClassifier;

impl LispKitClassifier {
    pub fn new() -> Self {
        LispKitClassifier
    }

    /// Returns `true` if the target contains a `*.sld` file (in either
    /// the pre-loaded manifests or the top-level-files list).
    fn has_sld_file(target: &Target) -> bool {
        // Check pre-loaded manifests.
        if target.manifests.iter().any(|m| m.name.ends_with(".sld")) {
            return true;
        }
        // Check the top-level directory listing.
        target.top_level_files.iter().any(|f| f.ends_with(".sld"))
    }
}

impl Analyzer for LispKitClassifier {
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
        Self::has_sld_file(target)
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        // Contribute every `.sld` manifest's content sha.
        let mut inputs = Vec::new();
        for m in &target.manifests {
            if m.name.ends_with(".sld") {
                inputs.push(FingerprintInput::FileContentSha(m.content_sha.clone()));
            }
        }
        inputs
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        let mut evidence: Vec<String> = Vec::new();

        // Collect matching manifest names.
        for m in &target.manifests {
            if m.name.ends_with(".sld") {
                evidence.push(m.name.clone());
            }
        }
        // Also probe top-level files if manifests didn't catch anything.
        if evidence.is_empty() {
            for f in &target.top_level_files {
                if f.ends_with(".sld") {
                    evidence.push(f.clone());
                }
            }
        }

        if evidence.is_empty() {
            return AnalyzerResult::Declines;
        }

        AnalyzerResult::Confident(Box::new(LispKitClassificationOutput {
            kind: "lispkit-package".into(),
            lifecycle_roles: vec!["build".into(), "runtime".into()],
            build_system: Some("lispkit".into()),
            role: None,
            language: "scheme".into(),
            evidence_fields: evidence.clone(),
            rationale: format!(
                "LispKit *.sld manifest signal present: {}.",
                evidence.join(", ")
            ),
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

    fn target_with_files(files: &[&str]) -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: files.iter().map(|n| (*n).to_string()).collect(),
        }
    }

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
                    content_sha: format!("sha-{name}"),
                })
                .collect(),
            top_level_files: files.iter().map(|(n, _)| (*n).to_string()).collect(),
        }
    }

    #[test]
    fn sld_in_top_level_files_yields_lispkit_package() {
        let target = target_with_files(&["core.sld", "utils.sld"]);
        let an = LispKitClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let lk = out
                    .as_any()
                    .downcast_ref::<LispKitClassificationOutput>()
                    .expect("output is LispKitClassificationOutput");
                assert_eq!(lk.kind, "lispkit-package");
                assert_eq!(lk.language, "scheme");
                assert_eq!(lk.build_system.as_deref(), Some("lispkit"));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn sld_in_manifests_yields_lispkit_package() {
        let target = target_with_manifests(&[("mylib.sld", "(define-library (mylib))")]);
        let an = LispKitClassifier::new();
        assert!(an.applies(&target));
        match an.analyse(&AnalysisContext::deterministic_only(), &target) {
            AnalyzerResult::Confident(out) => {
                let lk = out
                    .as_any()
                    .downcast_ref::<LispKitClassificationOutput>()
                    .unwrap();
                assert_eq!(lk.kind, "lispkit-package");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn applies_is_false_without_sld_files() {
        let target = target_with_files(&["Cargo.toml", "pyproject.toml"]);
        assert!(!LispKitClassifier::new().applies(&target));
    }

    #[test]
    fn fingerprint_inputs_from_manifests() {
        let target = target_with_manifests(&[
            ("mylib.sld", "(define-library (mylib))"),
            ("Cargo.toml", "[package]"),
        ]);
        let inputs = LispKitClassifier::new().fingerprint_inputs(&target);
        // Only the .sld manifest contributes to fingerprint inputs.
        assert_eq!(inputs.len(), 1);
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = LispKitClassifier::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L3);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }

    #[test]
    fn declines_without_sld_evidence() {
        let target = target_with_files(&["package.json"]);
        let an = LispKitClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        assert!(matches!(
            an.analyse(&ctx, &target),
            AnalyzerResult::Declines
        ));
    }
}
