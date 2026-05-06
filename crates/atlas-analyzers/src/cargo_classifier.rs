//! `Cargo.toml` classifier — the cheapest L3 analyser.
//!
//! Reads a candidate's `Cargo.toml` (already pre-loaded onto
//! [`Target::manifests`] by the engine), classifies the manifest
//! against three deterministic rules:
//!
//! 1. `[workspace]` present → `kind: workspace`.
//! 2. `[[bin]]` present (and no `[workspace]`) → `kind: rust-cli`.
//! 3. `[lib]` present (and no `[[bin]]`, no `[workspace]`) →
//!    `kind: rust-library`.
//!
//! Falls through to `Declines` when none of the rules fire — typically
//! when the candidate has no `Cargo.toml`. The trio mirrors the v1
//! `crates/atlas-engine/src/heuristics.rs::classify_deterministic`
//! behaviour but is reshaped to the [`Analyzer`] trait.
//!
//! `applies` is keyed on the presence of a `Cargo.toml` in the target's
//! pre-loaded manifests. Languages set is checked too: a candidate
//! whose L1 inferred languages set excludes `rust` is still allowed
//! through (Cargo manifests imply Rust regardless of upstream
//! inference).

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, FingerprintInput, Target};

/// Analyser id. Matches the wire form from design §6.6.
pub const ANALYZER_ID: &str = "cargo-toml-classifier";

/// Analyser version string. Bumped when the rule table changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// One Cargo classification verdict the analyser produces. The L3
/// adapter in `atlas-engine` downcasts the `Box<dyn StageOutput>`
/// back to this struct and translates onto the engine's
/// `Classification` type.
///
/// Keeping the struct in this crate keeps the trait shape closed to
/// `atlas-analyzers` types (so the engine can swap analysers without
/// taking on a reverse dep). The fields mirror `Classification`
/// closely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoClassificationOutput {
    /// Kebab-case kind matching `atlas_engine::ComponentKind`. Phase 1
    /// values: `workspace`, `rust-library`, `rust-cli`.
    pub kind: String,
    /// Lifecycle scopes (`build`, `runtime`, …). Empty unless the
    /// rule set populates them.
    pub lifecycle_roles: Vec<String>,
    pub build_system: Option<String>,
    pub role: Option<String>,
    /// Evidence fields fed back into the engine's rationale display.
    pub evidence_fields: Vec<String>,
    /// Human-readable rationale.
    pub rationale: String,
    /// Always true for Cargo classifications; here for symmetry with
    /// the engine's `Classification.is_boundary` field.
    pub is_boundary: bool,
}

/// The Cargo classifier itself. Stateless.
#[derive(Debug, Default)]
pub struct CargoClassifier;

impl CargoClassifier {
    pub fn new() -> Self {
        CargoClassifier
    }
}

impl Analyzer for CargoClassifier {
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
        target.manifest_by_name("Cargo.toml").is_some()
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        // Cargo classification is purely a function of `Cargo.toml`'s
        // contents. Other manifests under the candidate dir do not
        // affect the verdict, so contributing only the Cargo sha
        // keeps the cache tight.
        target
            .manifest_by_name("Cargo.toml")
            .map(|m| vec![FingerprintInput::FileContentSha(m.content_sha.clone())])
            .unwrap_or_default()
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        let Some(manifest) = target.manifest_by_name("Cargo.toml") else {
            // `applies` returned true but the file disappeared between
            // the predicate and the call — race. Decline rather than
            // erroring; the dispatcher will consult the next analyser.
            return AnalyzerResult::Declines;
        };

        let text = match std::str::from_utf8(&manifest.bytes) {
            Ok(s) => s,
            Err(e) => {
                return AnalyzerResult::Error(AnalyzerError::MalformedInput {
                    analyzer_id: ANALYZER_ID.into(),
                    message: format!("Cargo.toml is not valid UTF-8: {e}"),
                });
            }
        };

        let shape = match parse_cargo_toml(text) {
            Ok(s) => s,
            Err(e) => {
                return AnalyzerResult::Error(AnalyzerError::MalformedInput {
                    analyzer_id: ANALYZER_ID.into(),
                    message: format!("Cargo.toml parse failed: {e}"),
                });
            }
        };

        if shape.has_workspace_section {
            return AnalyzerResult::Confident(Box::new(CargoClassificationOutput {
                kind: "workspace".into(),
                lifecycle_roles: vec!["build".into()],
                build_system: Some("cargo".into()),
                role: None,
                evidence_fields: vec!["Cargo.toml:[workspace]".into()],
                rationale: "Cargo.toml declares a [workspace] section.".into(),
                is_boundary: true,
            }));
        }

        if shape.has_bin_section {
            return AnalyzerResult::Confident(Box::new(CargoClassificationOutput {
                kind: "rust-cli".into(),
                lifecycle_roles: vec!["build".into(), "runtime".into()],
                build_system: Some("cargo".into()),
                role: None,
                evidence_fields: vec!["Cargo.toml:[[bin]]".into()],
                rationale: "Cargo.toml declares a [[bin]] section.".into(),
                is_boundary: true,
            }));
        }

        if shape.has_lib_section {
            return AnalyzerResult::Confident(Box::new(CargoClassificationOutput {
                kind: "rust-library".into(),
                lifecycle_roles: vec!["build".into(), "runtime".into()],
                build_system: Some("cargo".into()),
                role: None,
                evidence_fields: vec!["Cargo.toml:[lib]".into()],
                rationale: "Cargo.toml declares a [lib] section with no [[bin]].".into(),
                is_boundary: true,
            }));
        }

        // Implicit-lib case: a `[package]` with `src/lib.rs` is a
        // library even without an explicit `[lib]`. The presence of
        // `src/lib.rs` is exposed via `Target.top_level_files`.
        if shape.has_package_section && target.top_level_files.iter().any(|n| n == "src") {
            // We cannot inspect inside `src/` without another
            // round-trip; rely on the engine's L1 manifest discovery
            // which has already classified `src/lib.rs` as a likely
            // library entrypoint — but Phase 1 keeps the conservative
            // rule that an explicit section is required. Decline
            // here so the LLM fallback can reason about it.
            return AnalyzerResult::Declines;
        }

        AnalyzerResult::Declines
    }
}

/// Minimal `Cargo.toml` shape sufficient for the three rules above.
/// Internal to this analyser. Public siblings of this type live in
/// `atlas-engine::manifest_parse` for L1's externals walk; we keep
/// the analyser's own parser local so the crate has no engine
/// dependency.
#[derive(Debug, Default, PartialEq, Eq)]
struct CargoTomlShape {
    has_workspace_section: bool,
    has_bin_section: bool,
    has_lib_section: bool,
    has_package_section: bool,
}

/// Parse a `Cargo.toml` against the shape above. Uses the `toml`
/// crate per memory `feedback_toml_parsing` — line scanning is
/// brittle and forbidden.
fn parse_cargo_toml(text: &str) -> Result<CargoTomlShape, toml::de::Error> {
    let value: toml::Value = toml::from_str(text)?;
    let table = match value.as_table() {
        Some(t) => t,
        None => return Ok(CargoTomlShape::default()),
    };
    Ok(CargoTomlShape {
        has_workspace_section: table.contains_key("workspace"),
        has_bin_section: table
            .get("bin")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        has_lib_section: table.contains_key("lib"),
        has_package_section: table.contains_key("package"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn target_with_manifest(text: &str) -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: vec![crate::TargetFile {
                name: "Cargo.toml".into(),
                relpath: PathBuf::from("Cargo.toml"),
                bytes: text.as_bytes().to_vec(),
                content_sha: format!("{:x}", text.len()),
            }],
            top_level_files: Vec::new(),
        }
    }

    fn assert_kind(text: &str, expected: &str) {
        let target = target_with_manifest(text);
        let an = CargoClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let cargo = out
                    .as_any()
                    .downcast_ref::<CargoClassificationOutput>()
                    .expect("output is CargoClassificationOutput");
                assert_eq!(cargo.kind, expected, "for input:\n{text}");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn workspace_kind_wins() {
        assert_kind(
            "[workspace]\nmembers = [\"a\"]\n[[bin]]\nname = \"x\"\n",
            "workspace",
        );
    }

    #[test]
    fn bin_section_yields_rust_cli() {
        assert_kind(
            "[package]\nname = \"x\"\n[[bin]]\nname = \"tool\"\n",
            "rust-cli",
        );
    }

    #[test]
    fn lib_section_yields_rust_library() {
        assert_kind(
            "[package]\nname = \"x\"\n[lib]\npath = \"src/lib.rs\"\n",
            "rust-library",
        );
    }

    #[test]
    fn applies_is_false_without_cargo_toml() {
        let target = Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        assert!(!CargoClassifier::new().applies(&target));
    }

    #[test]
    fn declines_when_only_package_section_present_no_lib_no_bin_no_src_dir() {
        // No explicit [lib] or [[bin]], no `src/` listed in top-level
        // files: decline so the LLM fallback gets a turn.
        let target = target_with_manifest("[package]\nname = \"x\"\n");
        let ctx = AnalysisContext::deterministic_only();
        let r = CargoClassifier::new().analyse(&ctx, &target);
        assert!(matches!(r, AnalyzerResult::Declines), "got {r:?}");
    }

    #[test]
    fn malformed_input_is_an_error_not_a_decline() {
        let target = target_with_manifest("[package\nname = \"x\"\n");
        let ctx = AnalysisContext::deterministic_only();
        let r = CargoClassifier::new().analyse(&ctx, &target);
        assert!(
            matches!(
                r,
                AnalyzerResult::Error(AnalyzerError::MalformedInput { .. })
            ),
            "got {r:?}"
        );
    }

    #[test]
    fn fingerprint_inputs_are_deterministic() {
        let target = target_with_manifest("[lib]\n");
        let a = CargoClassifier::new().fingerprint_inputs(&target);
        let b = CargoClassifier::new().fingerprint_inputs(&target);
        assert_eq!(a, b);
        assert_eq!(a.len(), 1);
    }
}
