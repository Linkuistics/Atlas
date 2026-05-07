//! C# L3 classifier (Atlas vNext Phase 2 PR-6).
//!
//! Sibling of [`crate::python_classifier`]: an L3 deterministic
//! analyser that emits `kind: csharp-project` when a candidate dir
//! carries a `*.csproj` manifest, or `kind: csharp-solution` when it
//! carries a `*.sln` manifest.
//!
//! ## Why separate from the legacy heuristics table
//!
//! `atlas-engine::heuristics` owns legacy rules for npm / pyproject
//! and bare-git. C# is added directly as a first-class analyser in the
//! registry (plan §4 PR-6) so the analyser id / version propagate to
//! per-component YAML and the registry sha includes the C# rule.
//!
//! ## Evidence
//!
//! `evidence_grade: Strong` — `*.csproj` / `*.sln` presence is
//! unambiguous for C# projects.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id. Matches the wire form a future
/// `analyzers.yaml` would carry.
pub const ANALYZER_ID: &str = "csharp-classifier";

/// Bumped when the rule table changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output shape mirroring [`crate::python_classifier::PythonClassificationOutput`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsharpClassificationOutput {
    /// `"csharp-project"` or `"csharp-solution"`.
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
pub struct CsharpClassifier;

impl CsharpClassifier {
    pub fn new() -> Self {
        CsharpClassifier
    }
}

impl Analyzer for CsharpClassifier {
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

    /// Applies when a `*.csproj` or `*.sln` manifest is present in
    /// the pre-loaded manifest list.
    fn applies(&self, target: &Target) -> bool {
        target
            .manifests
            .iter()
            .any(|f| f.name.ends_with(".csproj") || f.name.ends_with(".sln"))
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        // Contribute every csproj/sln manifest's content sha so that
        // a manifest change reshapes the L3 cache key.
        target
            .manifests
            .iter()
            .filter(|f| f.name.ends_with(".csproj") || f.name.ends_with(".sln"))
            .map(|f| FingerprintInput::FileContentSha(f.content_sha.clone()))
            .collect()
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        // Prefer *.sln over *.csproj (a solution file is a higher-level
        // boundary than a single project).
        let sln = target.manifests.iter().find(|f| f.name.ends_with(".sln"));
        if let Some(m) = sln {
            return AnalyzerResult::Confident(Box::new(CsharpClassificationOutput {
                kind: "csharp-solution".into(),
                lifecycle_roles: vec!["build".into(), "runtime".into()],
                build_system: Some("msbuild".into()),
                role: None,
                language: "csharp".into(),
                evidence_fields: vec![m.name.clone()],
                rationale: format!("Solution file `{}` present.", m.name),
                is_boundary: true,
            }));
        }

        let csproj = target
            .manifests
            .iter()
            .find(|f| f.name.ends_with(".csproj"));
        if let Some(m) = csproj {
            return AnalyzerResult::Confident(Box::new(CsharpClassificationOutput {
                kind: "csharp-project".into(),
                lifecycle_roles: vec!["build".into(), "runtime".into()],
                build_system: Some("msbuild".into()),
                role: None,
                language: "csharp".into(),
                evidence_fields: vec![m.name.clone()],
                rationale: format!("Project file `{}` present.", m.name),
                is_boundary: true,
            }));
        }

        AnalyzerResult::Declines
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
                    content_sha: format!("sha-{name}-{}", body.len()),
                })
                .collect(),
            top_level_files: files.iter().map(|(n, _)| (*n).to_string()).collect(),
        }
    }

    #[test]
    fn csproj_yields_csharp_project() {
        let target = target_with_manifests(&[("MyApp.csproj", "<Project />\n")]);
        let an = CsharpClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let cs = out
                    .as_any()
                    .downcast_ref::<CsharpClassificationOutput>()
                    .expect("output is CsharpClassificationOutput");
                assert_eq!(cs.kind, "csharp-project");
                assert_eq!(cs.language, "csharp");
                assert_eq!(cs.build_system.as_deref(), Some("msbuild"));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn sln_yields_csharp_solution() {
        let target = target_with_manifests(&[("MySolution.sln", "# VS solution\n")]);
        let an = CsharpClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let cs = out
                    .as_any()
                    .downcast_ref::<CsharpClassificationOutput>()
                    .unwrap();
                assert_eq!(cs.kind, "csharp-solution");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn sln_takes_precedence_over_csproj() {
        // A solution file at the root takes precedence over a project
        // file when both are present in the manifest list.
        let target = target_with_manifests(&[
            ("MySolution.sln", "# VS solution\n"),
            ("MyApp.csproj", "<Project />\n"),
        ]);
        let an = CsharpClassifier::new();
        match an.analyse(&AnalysisContext::deterministic_only(), &target) {
            AnalyzerResult::Confident(out) => {
                let cs = out
                    .as_any()
                    .downcast_ref::<CsharpClassificationOutput>()
                    .unwrap();
                assert_eq!(cs.kind, "csharp-solution");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn applies_is_false_without_csharp_manifest() {
        let target = target_with_manifests(&[("Cargo.toml", "[package]\n")]);
        assert!(!CsharpClassifier::new().applies(&target));
    }

    #[test]
    fn fingerprint_inputs_one_per_csharp_manifest() {
        let target = target_with_manifests(&[
            ("MyApp.csproj", "<Project />\n"),
            ("package.json", "{}"),
        ]);
        let inputs = CsharpClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 1);
        assert!(matches!(inputs[0], FingerprintInput::FileContentSha(_)));
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = CsharpClassifier::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L3);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }
}
