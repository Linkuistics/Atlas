//! TypeScript / JavaScript L3 classifier.
//!
//! L3 deterministic analyser that emits
//! `kind: typescript-package` when a candidate
//! carries both a `package.json` and a `tsconfig.json`, and
//! `kind: javascript-package` when only a `package.json` is present.
//!
//! Phase 2 PR-1 introduces both kinds so the new TS/JS surface
//! analyser ([`crate::ts_js_surface_analyzer`]) has a stable
//! classification target. The legacy `node-library` / `node-cli`
//! deterministic rules in `atlas-engine::heuristics` continue to
//! fire when this classifier declines (e.g. when the manifest carries
//! a `bin` field — see the rule precedence note below).
//!
//! # Rule table
//!
//! 1. `package.json` + `tsconfig.json` present → `typescript-package`.
//!    Always wins over the legacy node rules: the explicit TypeScript
//!    config makes this an unambiguous TS package.
//! 2. `package.json` present, no `tsconfig.json`, and the manifest
//!    carries no `bin`, `main`, or `exports` field →
//!    `javascript-package`. This covers bare-`package.json` projects
//!    (e.g. workspace roots with no entry-point declared); the legacy
//!    `node-library` / `node-cli` rules in `atlas-engine::heuristics`
//!    handle the entry-point cases.
//! 3. Otherwise → `Declines` (the L3 dispatcher consults the next
//!    analyser; the legacy node rules pick up `bin` / `main` /
//!    `exports` structures the new kinds do not yet model).
//!
//! Every rule requires `package.json` to parse as JSON. A malformed
//! manifest is an `AnalyzerResult::Error` rather than a decline so the
//! dispatcher surfaces the parse failure to the user.
//!
//! # Why these are new kinds (not `node-library` / `node-cli`)
//!
//! `node-library` / `node-cli` describe Node.js packaging shape (the
//! presence of `main` / `exports` / `bin`). The new kinds pin the
//! source language explicitly, which is what the surface analyser
//! cares about: it runs `swc_ecma_parser` in TypeScript mode for
//! `typescript-package` and JavaScript mode for `javascript-package`.
//! The two vocabularies coexist; PR-1 introduces these new kinds and
//! leaves the legacy ones untouched. A future PR may unify the two.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id. Matches the wire form a future
/// `analyzers.yaml` would carry (design §6.6).
pub const ANALYZER_ID: &str = "ts-js-classifier";

/// Bumped when the rule table changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output of the TS/JS classifier. Mirrors sibling classifiers'
/// `*ClassificationOutput` structs so the engine's L3 adapter can
/// downcast each output uniformly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TsJsClassificationOutput {
    /// Kebab-case kind: `typescript-package` or `javascript-package`.
    pub kind: String,
    /// Lifecycle scopes applicable to the package. Phase 2 PR-1 emits
    /// `["build", "runtime"]` for both kinds.
    pub lifecycle_roles: Vec<String>,
    /// Build system identifier (`npm`).
    pub build_system: Option<String>,
    /// Optional role; PR-1 leaves this `None`.
    pub role: Option<String>,
    /// Source language for the surface analyser to key on (`typescript`
    /// or `javascript`).
    pub language: String,
    /// Evidence fields fed into the engine rationale.
    pub evidence_fields: Vec<String>,
    /// Human-readable rationale.
    pub rationale: String,
    /// Always true — both kinds are component boundaries.
    pub is_boundary: bool,
}

/// The TS/JS classifier itself. Stateless.
#[derive(Debug, Default)]
pub struct TsJsClassifier;

impl TsJsClassifier {
    pub fn new() -> Self {
        TsJsClassifier
    }
}

impl Analyzer for TsJsClassifier {
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
        // The cheapest applies predicate: presence of a top-level
        // `package.json`. The `analyse` path probes for `tsconfig.json`
        // separately to distinguish the two kinds.
        target.manifest_by_name("package.json").is_some()
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        let mut inputs = Vec::new();
        if let Some(pkg) = target.manifest_by_name("package.json") {
            inputs.push(FingerprintInput::FileContentSha(pkg.content_sha.clone()));
        }
        if let Some(ts) = target.manifest_by_name("tsconfig.json") {
            inputs.push(FingerprintInput::FileContentSha(ts.content_sha.clone()));
        }
        inputs
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        let Some(pkg) = target.manifest_by_name("package.json") else {
            // `applies` returned true but the manifest disappeared —
            // race. Decline so the dispatcher can consult the next.
            return AnalyzerResult::Declines;
        };

        // Parse package.json. Any shape qualifies; we look at `bin` /
        // `main` / `exports` to know whether to defer to the legacy
        // `node-cli` / `node-library` deterministic rules in
        // `atlas-engine::heuristics`. A malformed manifest is an Error,
        // not a Decline, so the dispatcher surfaces the parse failure
        // to the user rather than silently degrading.
        let pkg_text = match std::str::from_utf8(&pkg.bytes) {
            Ok(s) => s,
            Err(e) => {
                return AnalyzerResult::Error(AnalyzerError::MalformedInput {
                    analyzer_id: ANALYZER_ID.into(),
                    message: format!("package.json is not valid UTF-8: {e}"),
                });
            }
        };
        let pkg_value: serde_json::Value = match serde_json::from_str(pkg_text) {
            Ok(v) => v,
            Err(e) => {
                return AnalyzerResult::Error(AnalyzerError::MalformedInput {
                    analyzer_id: ANALYZER_ID.into(),
                    message: format!("package.json parse failed: {e}"),
                });
            }
        };

        let has_tsconfig = target.manifest_by_name("tsconfig.json").is_some();

        // Rule 1: package.json + tsconfig.json → typescript-package.
        // Always wins, regardless of `bin` / `main` / `exports`: the
        // explicit TS config is unambiguous, and the legacy node rules
        // would mis-label the package's source language as JS otherwise.
        if has_tsconfig {
            return AnalyzerResult::Confident(Box::new(TsJsClassificationOutput {
                kind: "typescript-package".into(),
                lifecycle_roles: vec!["build".into(), "runtime".into()],
                build_system: Some("npm".into()),
                role: None,
                language: "typescript".into(),
                evidence_fields: vec!["package.json".into(), "tsconfig.json".into()],
                rationale: "package.json with adjacent tsconfig.json declares a TypeScript \
                            package."
                    .into(),
                is_boundary: true,
            }));
        }

        // Rule 2: defer to legacy node-cli / node-library rules when
        // the manifest declares a packaging shape (`bin` / `main` /
        // `exports`). This preserves the Phase 1 vocabulary for those
        // cases; the new `javascript-package` kind only fires for the
        // bare-`package.json` case the legacy rules would otherwise
        // hand off to the LLM.
        let pkg_obj = pkg_value.as_object();
        let has_packaging_shape = pkg_obj
            .map(|o| o.contains_key("bin") || o.contains_key("main") || o.contains_key("exports"))
            .unwrap_or(false);
        if has_packaging_shape {
            return AnalyzerResult::Declines;
        }

        // Rule 3: bare `package.json` (no tsconfig, no packaging shape)
        // → javascript-package.
        AnalyzerResult::Confident(Box::new(TsJsClassificationOutput {
            kind: "javascript-package".into(),
            lifecycle_roles: vec!["build".into(), "runtime".into()],
            build_system: Some("npm".into()),
            role: None,
            language: "javascript".into(),
            evidence_fields: vec!["package.json".into()],
            rationale: "package.json present with no tsconfig.json and no `bin` / `main` / \
                        `exports` field declares a bare JavaScript package."
                .into(),
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
    fn package_json_with_tsconfig_yields_typescript_package() {
        let target = target_with_manifests(&[
            ("package.json", "{\"name\":\"x\"}"),
            ("tsconfig.json", "{\"compilerOptions\":{}}"),
        ]);
        let an = TsJsClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let ts = out
                    .as_any()
                    .downcast_ref::<TsJsClassificationOutput>()
                    .expect("output is TsJsClassificationOutput");
                assert_eq!(ts.kind, "typescript-package");
                assert_eq!(ts.language, "typescript");
                assert!(ts.evidence_fields.contains(&"tsconfig.json".to_string()));
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn bare_package_json_without_tsconfig_yields_javascript_package() {
        // No `bin` / `main` / `exports` → falls through to
        // `javascript-package`. The legacy node rules would have
        // declined too (no packaging shape), so the LLM-classify
        // analyser used to handle this case; PR-1 makes it
        // deterministic.
        let target = target_with_manifests(&[("package.json", "{\"name\":\"x\"}")]);
        let an = TsJsClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let ts = out
                    .as_any()
                    .downcast_ref::<TsJsClassificationOutput>()
                    .expect("output is TsJsClassificationOutput");
                assert_eq!(ts.kind, "javascript-package");
                assert_eq!(ts.language, "javascript");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn package_json_with_main_declines_so_legacy_node_library_wins() {
        // The TS/JS classifier defers to the legacy node-library rule
        // (which ALSO emits `node-library`). Without this defer, the
        // legacy semantics would silently flip from `node-library` to
        // `javascript-package` — Phase 2 PR-1 keeps backward
        // compatibility for the existing kinds.
        let target =
            target_with_manifests(&[("package.json", "{\"name\":\"x\",\"main\":\"index.js\"}")]);
        let an = TsJsClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        let r = an.analyse(&ctx, &target);
        assert!(matches!(r, AnalyzerResult::Declines), "got {r:?}");
    }

    #[test]
    fn package_json_with_bin_declines_so_legacy_node_cli_wins() {
        let target =
            target_with_manifests(&[("package.json", "{\"name\":\"x\",\"bin\":\"cli.js\"}")]);
        let an = TsJsClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        let r = an.analyse(&ctx, &target);
        assert!(matches!(r, AnalyzerResult::Declines), "got {r:?}");
    }

    #[test]
    fn package_json_with_exports_declines_so_legacy_rules_win() {
        let target = target_with_manifests(&[(
            "package.json",
            "{\"name\":\"x\",\"exports\":\"./index.js\"}",
        )]);
        let an = TsJsClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        let r = an.analyse(&ctx, &target);
        assert!(matches!(r, AnalyzerResult::Declines), "got {r:?}");
    }

    #[test]
    fn package_json_with_main_plus_tsconfig_still_typescript_package() {
        // Even when packaging fields are present, an adjacent
        // `tsconfig.json` makes this an unambiguous TS package and
        // wins over the legacy node rules.
        let target = target_with_manifests(&[
            ("package.json", "{\"name\":\"x\",\"main\":\"index.js\"}"),
            ("tsconfig.json", "{}"),
        ]);
        let an = TsJsClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let ts = out
                    .as_any()
                    .downcast_ref::<TsJsClassificationOutput>()
                    .expect("output is TsJsClassificationOutput");
                assert_eq!(ts.kind, "typescript-package");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn applies_is_false_without_package_json() {
        let target = target_with_manifests(&[("tsconfig.json", "{}")]);
        assert!(!TsJsClassifier::new().applies(&target));
    }

    #[test]
    fn malformed_package_json_is_an_error_not_a_decline() {
        let target = target_with_manifests(&[("package.json", "{ broken")]);
        let an = TsJsClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        let r = an.analyse(&ctx, &target);
        assert!(
            matches!(
                r,
                AnalyzerResult::Error(AnalyzerError::MalformedInput { .. })
            ),
            "got {r:?}"
        );
    }

    #[test]
    fn fingerprint_inputs_include_both_manifests_when_present() {
        let target = target_with_manifests(&[("package.json", "{}"), ("tsconfig.json", "{}")]);
        let inputs = TsJsClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 2);
    }

    #[test]
    fn fingerprint_inputs_are_just_package_json_when_tsconfig_absent() {
        let target = target_with_manifests(&[("package.json", "{}")]);
        let inputs = TsJsClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 1);
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = TsJsClassifier::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L3);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }
}
