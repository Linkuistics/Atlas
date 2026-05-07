//! Dart / Flutter L3 classifier (Atlas vNext Phase 2 PR-7).
//!
//! Sibling of [`crate::python_classifier`] and [`crate::cargo_classifier`]:
//! an L3 deterministic analyser that emits `kind: dart-package` or
//! `kind: flutter-package` whenever a candidate dir carries a
//! `pubspec.yaml` manifest.
//!
//! ## Discrimination rule
//!
//! - `pubspec.yaml` present + no `flutter:` block → `dart-package`.
//! - `pubspec.yaml` present + `flutter:` top-level key → `flutter-package`.
//!
//! The `flutter:` block is the canonical Flutter SDK marker: the Flutter
//! tool adds it when scaffolding a project, and it is required for the
//! Flutter build system to run.
//!
//! ## Evidence
//!
//! `evidence_grade: Strong` — manifest presence is unambiguous.
//! `evidence_fields` lists the manifest basename plus the flutter signal
//! so the per-component rationale is auditable.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id. Matches the wire form a future `analyzers.yaml` would carry.
pub const ANALYZER_ID: &str = "dart-classifier";

/// Bumped when the rule table changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output shape mirroring [`crate::python_classifier::PythonClassificationOutput`].
/// The L3 adapter downcasts the `Box<dyn StageOutput>` to this struct
/// and translates onto `Classification`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DartClassificationOutput {
    /// `"dart-package"` or `"flutter-package"`.
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
pub struct DartClassifier;

impl DartClassifier {
    pub fn new() -> Self {
        DartClassifier
    }
}

impl Analyzer for DartClassifier {
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
        target.manifest_by_name("pubspec.yaml").is_some()
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        let mut inputs = Vec::new();
        if let Some(m) = target.manifest_by_name("pubspec.yaml") {
            inputs.push(FingerprintInput::FileContentSha(m.content_sha.clone()));
        }
        inputs
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        let Some(pubspec) = target.manifest_by_name("pubspec.yaml") else {
            return AnalyzerResult::Declines;
        };

        let text = match std::str::from_utf8(&pubspec.bytes) {
            Ok(t) => t,
            Err(_) => {
                // Non-UTF-8 pubspec — treat as Dart package, we can't
                // parse the flutter block.
                return AnalyzerResult::Confident(Box::new(DartClassificationOutput {
                    kind: "dart-package".into(),
                    lifecycle_roles: vec!["build".into(), "runtime".into()],
                    build_system: Some("pub".into()),
                    role: None,
                    language: "dart".into(),
                    evidence_fields: vec!["pubspec.yaml".into()],
                    rationale:
                        "Dart manifest pubspec.yaml present (non-UTF-8, no flutter block detected)."
                            .into(),
                    is_boundary: true,
                }));
            }
        };

        let has_flutter_block = has_flutter_top_level_key(text);

        let (kind, rationale, mut evidence_fields) = if has_flutter_block {
            (
                "flutter-package",
                "Flutter manifest pubspec.yaml with flutter: block detected.",
                vec!["pubspec.yaml".into(), "flutter:".into()],
            )
        } else {
            (
                "dart-package",
                "Dart manifest pubspec.yaml present, no flutter: block.",
                vec!["pubspec.yaml".into()],
            )
        };

        // Also record the package name as evidence if we can extract it.
        if let Some(name) = extract_pubspec_name(text) {
            evidence_fields.push(format!("name:{name}"));
        }

        AnalyzerResult::Confident(Box::new(DartClassificationOutput {
            kind: kind.into(),
            lifecycle_roles: vec!["build".into(), "runtime".into()],
            build_system: Some("pub".into()),
            role: None,
            language: "dart".into(),
            evidence_fields,
            rationale: rationale.into(),
            is_boundary: true,
        }))
    }
}

/// Returns `true` when the pubspec YAML text contains a `flutter:` key
/// at the top level (i.e., not indented).
///
/// We use a line-scan rather than a full YAML parse to avoid pulling in
/// `serde_yaml` as a dep of `atlas-analyzers`. The classifier only
/// needs a presence check, not a value parse.
///
/// Canonical `flutter:` line forms in the wild:
/// - `flutter:` (block mapping, no inline value)
/// - `flutter: sdk: flutter` (inline flow value, unusual)
///
/// A false positive from a comment (`# flutter:`) or an indented
/// child key (`  flutter:`) is excluded by checking that the line
/// starts with `flutter:` (zero leading whitespace, not in a comment).
fn has_flutter_top_level_key(text: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim_end();
        // Skip comments.
        if trimmed.starts_with('#') {
            continue;
        }
        // Top-level key: zero leading whitespace, starts with "flutter:".
        if trimmed.starts_with("flutter:") {
            return true;
        }
    }
    false
}

/// Best-effort extraction of the `name:` field from pubspec.yaml.
/// Only recognises the simple `name: <identifier>` top-level form.
/// Returns `None` for malformed or missing names — the caller uses
/// it only for diagnostic evidence fields.
fn extract_pubspec_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            let name = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TargetFile;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn target_with_pubspec(body: &str) -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: vec![TargetFile {
                name: "pubspec.yaml".into(),
                relpath: PathBuf::from("pubspec.yaml"),
                bytes: body.as_bytes().to_vec(),
                content_sha: format!("sha-{}", body.len()),
            }],
            top_level_files: vec!["pubspec.yaml".to_string()],
        }
    }

    fn target_without_pubspec() -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: vec![],
            top_level_files: vec![],
        }
    }

    #[test]
    fn dart_package_without_flutter_block() {
        let pubspec = "name: dart_pkg\nversion: 0.1.0\n\ndependencies:\n  meta: any\n";
        let target = target_with_pubspec(pubspec);
        let an = DartClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let dart = out
                    .as_any()
                    .downcast_ref::<DartClassificationOutput>()
                    .expect("output is DartClassificationOutput");
                assert_eq!(dart.kind, "dart-package");
                assert_eq!(dart.language, "dart");
                assert_eq!(dart.build_system.as_deref(), Some("pub"));
                assert!(dart.is_boundary);
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn flutter_package_with_flutter_block() {
        let pubspec = "name: flutter_app\nversion: 1.0.0\n\nflutter:\n  uses-material-design: true\n\ndependencies:\n  flutter:\n    sdk: flutter\n";
        let target = target_with_pubspec(pubspec);
        let an = DartClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let dart = out
                    .as_any()
                    .downcast_ref::<DartClassificationOutput>()
                    .unwrap();
                assert_eq!(dart.kind, "flutter-package");
                assert!(dart.evidence_fields.contains(&"flutter:".to_string()));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn flutter_key_in_comment_is_not_detected() {
        // A `# flutter:` comment must not trigger the flutter-package verdict.
        let pubspec = "name: dart_pkg\nversion: 0.1.0\n# flutter: would be here if it were Flutter\ndependencies:\n  meta: any\n";
        let target = target_with_pubspec(pubspec);
        let an = DartClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let dart = out
                    .as_any()
                    .downcast_ref::<DartClassificationOutput>()
                    .unwrap();
                assert_eq!(
                    dart.kind, "dart-package",
                    "comment must not trigger flutter-package"
                );
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn flutter_key_indented_is_not_detected_as_top_level() {
        // An indented `  flutter:` in the dependencies block must not
        // trigger the top-level `flutter:` detection.
        let pubspec =
            "name: dart_pkg\nversion: 0.1.0\ndependencies:\n  flutter:\n    sdk: flutter\n";
        let target = target_with_pubspec(pubspec);
        let an = DartClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let dart = out
                    .as_any()
                    .downcast_ref::<DartClassificationOutput>()
                    .unwrap();
                assert_eq!(
                    dart.kind, "dart-package",
                    "indented flutter: must not trigger flutter-package"
                );
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn applies_is_false_without_pubspec() {
        let target = target_without_pubspec();
        assert!(!DartClassifier::new().applies(&target));
    }

    #[test]
    fn fingerprint_inputs_includes_pubspec_sha() {
        let target = target_with_pubspec("name: x\n");
        let inputs = DartClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 1);
        match &inputs[0] {
            FingerprintInput::FileContentSha(sha) => assert!(!sha.is_empty()),
            _ => panic!("expected FileContentSha"),
        }
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = DartClassifier::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L3);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }

    #[test]
    fn dart_package_name_extracted_as_evidence_field() {
        let pubspec = "name: my_dart_lib\nversion: 0.1.0\n";
        let target = target_with_pubspec(pubspec);
        let an = DartClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let dart = out
                    .as_any()
                    .downcast_ref::<DartClassificationOutput>()
                    .unwrap();
                assert!(
                    dart.evidence_fields
                        .iter()
                        .any(|f| f.contains("my_dart_lib")),
                    "package name must appear in evidence_fields; got {:?}",
                    dart.evidence_fields
                );
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }
}
