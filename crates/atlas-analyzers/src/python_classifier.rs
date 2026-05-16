//! Python L3 classifier (Atlas vNext Phase 2 PR-3).
//!
//! Sibling of [`crate::ts_js_classifier`]: an L3 deterministic analyser that
//! emits `kind: python-package` whenever a candidate dir carries one
//! of the three canonical Python manifest signals:
//!
//! - `pyproject.toml` (PEP 517 / 518 / 621, Poetry, Hatch, uv).
//! - `setup.py` (legacy distutils / setuptools shape).
//! - `requirements.txt` (bare-Python without a packaging story —
//!   still a useful component boundary).
//!
//! The pre-PR-3 vocabulary used `python-library` / `python-app`
//! (emitted by `atlas-engine::heuristics::rule_pyproject_toml`); the
//! new `python-package` kind sits alongside those, distinguished by
//! the analyser-id sentinel on the resulting `Classification`. The
//! legacy heuristic rule still fires when this analyser declines
//! (e.g. on a manifest the analyser registry has not yet been
//! populated against).
//!
//! ## Why this is a separate analyser
//!
//! The plan §4 PR-3 brief calls out a Python *L3 classifier* as a
//! sibling of the new `python-package` kind in `schema.rs`. Lifting
//! the rule out of `atlas-engine::heuristics` into a standalone
//! analyser closes the loop: the registry's deterministic pass owns
//! the verdict, the analyser-registry sha contributes to L3 cache
//! invalidation, and the analyser-id propagates to per-component
//! YAML via PR-4's plumbing.
//!
//! ## Phase 2 evidence
//!
//! `evidence_grade: Strong` — manifest presence is unambiguous.
//! `evidence_fields` lists the matching manifest basename so the
//! per-component rationale is auditable.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id. Matches the wire form a future
/// `analyzers.yaml` would carry.
pub const ANALYZER_ID: &str = "python-classifier";

/// Bumped when the rule table changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output shape mirroring sibling classifiers' `*ClassificationOutput`
/// structs. The L3 adapter downcasts the `Box<dyn StageOutput>` to
/// this struct and translates onto `Classification`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonClassificationOutput {
    /// Always `"python-package"` for Phase 2 PR-3. Future PRs may
    /// distinguish `python-library` vs `python-app` by inspecting the
    /// manifest's entry-point fields.
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
pub struct PythonClassifier;

impl PythonClassifier {
    pub fn new() -> Self {
        PythonClassifier
    }
}

impl Analyzer for PythonClassifier {
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
        for name in ["pyproject.toml", "setup.py", "requirements.txt"] {
            if target.manifest_by_name(name).is_some() {
                return true;
            }
        }
        false
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        // Contribute every Python manifest's content sha. A
        // pyproject.toml change reshapes the L3 cache key for this
        // analyser without forcing the whole analyser-registry sha
        // to flip.
        let mut inputs = Vec::new();
        for name in ["pyproject.toml", "setup.py", "requirements.txt"] {
            if let Some(m) = target.manifest_by_name(name) {
                inputs.push(FingerprintInput::FileContentSha(m.content_sha.clone()));
            }
        }
        inputs
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        // Walk the recognised manifests in canonical priority order
        // (pyproject.toml first — the PEP-621 source of truth).
        let mut evidence: Vec<String> = Vec::new();
        let mut build_system: Option<String> = None;
        for name in ["pyproject.toml", "setup.py", "requirements.txt"] {
            if target.manifest_by_name(name).is_some() {
                evidence.push((*name).to_string());
                if build_system.is_none() {
                    build_system = match name {
                        "pyproject.toml" => Some("pyproject".into()),
                        "setup.py" => Some("setuptools".into()),
                        "requirements.txt" => Some("pip".into()),
                        _ => None,
                    };
                }
            }
        }
        if evidence.is_empty() {
            return AnalyzerResult::Declines;
        }

        AnalyzerResult::Confident(Box::new(PythonClassificationOutput {
            kind: "python-package".into(),
            lifecycle_roles: vec!["build".into(), "runtime".into()],
            build_system,
            role: None,
            language: "python".into(),
            evidence_fields: evidence.clone(),
            rationale: format!("Python manifest signal present: {}.", evidence.join(", ")),
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
    fn pyproject_toml_yields_python_package() {
        let target = target_with_manifests(&[("pyproject.toml", "[project]\nname=\"x\"\n")]);
        let an = PythonClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let py = out
                    .as_any()
                    .downcast_ref::<PythonClassificationOutput>()
                    .expect("output is PythonClassificationOutput");
                assert_eq!(py.kind, "python-package");
                assert_eq!(py.language, "python");
                assert_eq!(py.build_system.as_deref(), Some("pyproject"));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn setup_py_yields_python_package_setuptools_build_system() {
        let target = target_with_manifests(&[("setup.py", "from setuptools import setup\n")]);
        let an = PythonClassifier::new();
        match an.analyse(&AnalysisContext::deterministic_only(), &target) {
            AnalyzerResult::Confident(out) => {
                let py = out
                    .as_any()
                    .downcast_ref::<PythonClassificationOutput>()
                    .unwrap();
                assert_eq!(py.kind, "python-package");
                assert_eq!(py.build_system.as_deref(), Some("setuptools"));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn requirements_txt_yields_python_package_pip_build_system() {
        let target = target_with_manifests(&[("requirements.txt", "requests==2\n")]);
        let an = PythonClassifier::new();
        match an.analyse(&AnalysisContext::deterministic_only(), &target) {
            AnalyzerResult::Confident(out) => {
                let py = out
                    .as_any()
                    .downcast_ref::<PythonClassificationOutput>()
                    .unwrap();
                assert_eq!(py.kind, "python-package");
                assert_eq!(py.build_system.as_deref(), Some("pip"));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn pyproject_takes_precedence_over_setup_py_for_build_system() {
        let target = target_with_manifests(&[
            ("pyproject.toml", "[project]\nname=\"x\"\n"),
            ("setup.py", "from setuptools import setup\n"),
        ]);
        let an = PythonClassifier::new();
        match an.analyse(&AnalysisContext::deterministic_only(), &target) {
            AnalyzerResult::Confident(out) => {
                let py = out
                    .as_any()
                    .downcast_ref::<PythonClassificationOutput>()
                    .unwrap();
                assert_eq!(py.build_system.as_deref(), Some("pyproject"));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn applies_is_false_without_any_python_manifest() {
        let target = target_with_manifests(&[("Cargo.toml", "[package]\n")]);
        assert!(!PythonClassifier::new().applies(&target));
    }

    #[test]
    fn fingerprint_inputs_one_per_present_manifest() {
        let target = target_with_manifests(&[
            ("pyproject.toml", "[project]\n"),
            ("requirements.txt", "x\n"),
        ]);
        let inputs = PythonClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 2);
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = PythonClassifier::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L3);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }
}
