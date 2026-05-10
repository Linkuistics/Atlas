//! `<output>/.atlas/analyzers.yaml` schema (Atlas vNext Phase 1).
//!
//! Declarative analyser registry overrides. The shipped registry has
//! built-in defaults (Cargo classifier, Dockerfile classifier,
//! LLM-classify fallback in Phase 1; more in Phase 2). The
//! `analyzers.yaml` file extends or overrides them per workspace
//! (design §5.2, §6.6, §7).
//!
//! This module is types-only; the dispatcher and registry live in
//! `crates/atlas-analyzers` (PR-5). Phase 1 ships
//! `schema_version: 1`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Schema version for `analyzers.yaml`.
pub const ANALYZERS_SCHEMA_VERSION: u32 = 1;

/// Which L-stage an analyser plugs into. Closed kebab-case enum so a
/// typo in a YAML registry file fails loudly.
///
/// Variants serialise as lowercase strings (`l1`, `l2`, …, `l9`) to
/// match the wire form used in `analyzers.yaml` (see design §6.6).
/// `kebab-case` reduces to lowercase here because each variant is a
/// single token without internal word breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    L1,
    L2,
    L3,
    L4,
    L5,
    L6,
    L7,
    L8,
    L9,
}

/// Cost class of an analyser. The dispatcher picks the cheapest
/// applicable analyser whose confidence reaches the configured
/// threshold (design §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CostClass {
    /// In-process pure-Rust parser; microseconds.
    DeterministicCheap,
    /// Subprocess call to a language-specific tool (e.g. rust-analyzer);
    /// seconds.
    DeterministicExpensive,
    /// LLM call against a small / fast model (haiku-class).
    LlmCheap,
    /// LLM call against a large / slow model (sonnet-/opus-class).
    LlmExpensive,
}

/// Confidence shape an analyser declares. `Binary` analysers always
/// produce `Confident` or `Error`; `Graded` analysers attach a
/// `[0, 1]` confidence; `Declines` analysers are fallthrough-only
/// (e.g. the LLM classify analyser declines anything a deterministic
/// analyser already handled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    Binary,
    Graded,
    Declines,
}

/// Process-boundary transport for the analyser. Phase 1 ships only
/// `in-process`; Phase 2 adds `subprocess` for crash isolation
/// (design §5.2, §7.2).
///
/// On disk this is a plain kebab-case scalar (`transport: in-process`
/// or `transport: subprocess`); the subprocess command/timeout live
/// under a sibling `subprocess:` map on the `AnalyzerSpec`, per the
/// design §6.6 example. See [`SubprocessConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    /// Rust trait object inside the engine process.
    InProcess,
    /// Spawned subprocess speaking line-delimited JSON over stdio
    /// (design §7.2). Configuration is carried in
    /// [`AnalyzerSpec::subprocess`].
    Subprocess,
}

/// Subprocess transport configuration — appears as a sibling
/// `subprocess:` map on an [`AnalyzerSpec`] when (and only when) its
/// `transport` is [`Transport::Subprocess`].
///
/// Phase 2 makes [`SubprocessConfig::binary_sha`] load-bearing: the
/// engine records the analyser binary's sha256 at registry
/// construction time and feeds it into every L-stage cache fingerprint
/// via `FingerprintBuilder::add_analyzer_binary_sha` (tag `0x06`). A
/// rebuilt analyser binary therefore invalidates downstream cache
/// entries even when the YAML spec is byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubprocessConfig {
    /// argv of the subprocess (analyser binary + args). Resolved
    /// against `$PATH` and the workspace `override_search`.
    pub command: Vec<String>,
    /// Hard timeout per request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// Lowercase 64-character sha256 hex of the analyser binary's
    /// contents. Computed once by the engine when the registry is
    /// constructed and contributed to every L-stage cache fingerprint
    /// the analyser participates in. Optional in the wire form (a
    /// hand-authored `analyzers.yaml` may omit it; the engine fills
    /// it in from the binary on disk before constructing the
    /// `SubprocessAnalyzerProxy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha: Option<String>,
}

/// Applicability predicate for an analyser. Phase 1 supports four
/// shapes (design §6.6 example): file globs, language tags, manifest
/// types, and unconditional `always`. Modelled as a struct of
/// optional fields so the YAML reads naturally — combining a
/// `file_globs` filter with a `languages` filter is the conjunction.
///
/// All four fields are optional; an analyser whose predicate has
/// `always: true` (and nothing else set) is the LLM-fallback shape.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApplicabilityPredicate {
    /// glob patterns matched against file paths relative to the
    /// component root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_globs: Vec<String>,
    /// Language tags (`rust`, `typescript`, …). Matches when the
    /// target's language set intersects this list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Manifest-type tags (`cargo`, `npm`, `helm`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manifest_types: Vec<String>,
    /// Unconditional applicability. Used by the LLM-fallback analyser
    /// in combination with `confidence: declines` so it only runs
    /// when no other analyser was confident.
    #[serde(default, skip_serializing_if = "is_false")]
    pub always: bool,
}

/// Skip-serialise helper for the `always` boolean — keeps the
/// emitted YAML small for the common case (`always: false`, written
/// as the field's absence).
fn is_false(b: &bool) -> bool {
    !*b
}

/// One analyser declaration. Mirrors the design §6.6 example shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzerSpec {
    /// Stable analyser id (`cargo-toml-classifier`, `dockerfile-l1`,
    /// …). Contributes to fingerprint inputs (design §8.1).
    pub id: String,
    pub stage: Stage,
    pub applicability: ApplicabilityPredicate,
    pub cost_class: CostClass,
    /// Optional. Defaults to `Binary` for in-process deterministic
    /// analysers; emit explicitly for LLM analysers and any analyser
    /// whose semantics include `declines`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Process boundary (in-process or subprocess). Emitted as
    /// `transport: in-process` or `transport: subprocess`. The
    /// subprocess configuration lives under the sibling
    /// [`AnalyzerSpec::subprocess`] map (design §6.6 example).
    pub transport: Transport,
    /// Subprocess configuration. Required when `transport: subprocess`,
    /// must be absent when `transport: in-process`. See
    /// [`AnalyzerSpec::validate`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subprocess: Option<SubprocessConfig>,
    /// Analyser version string. Free-form; the dispatcher includes
    /// this in cache fingerprints so a version bump invalidates
    /// cached outputs.
    pub version: String,
}

/// Reasons an [`AnalyzerSpec`] can be semantically inconsistent.
/// The on-disk format permits the inconsistency to be expressed
/// (`transport: in-process` with a stray `subprocess:` map, or
/// `transport: subprocess` with no map); validation is a runtime
/// check on top of `serde`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalyzerSpecValidationError {
    /// `transport: subprocess` but `subprocess:` map is absent.
    SubprocessConfigMissing,
    /// `transport: in-process` but `subprocess:` map is present.
    SubprocessConfigUnexpected,
}

impl std::fmt::Display for AnalyzerSpecValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SubprocessConfigMissing => f.write_str(
                "analyzer spec has `transport: subprocess` but no `subprocess:` config map",
            ),
            Self::SubprocessConfigUnexpected => f.write_str(
                "analyzer spec has `transport: in-process` but a `subprocess:` config map is set",
            ),
        }
    }
}

impl std::error::Error for AnalyzerSpecValidationError {}

impl AnalyzerSpec {
    /// Cross-check `transport` against `subprocess`. The on-disk
    /// schema deliberately keeps these as independent fields (so the
    /// YAML reads naturally per design §6.6); this method enforces
    /// the constraint that links them.
    pub fn validate(&self) -> Result<(), AnalyzerSpecValidationError> {
        match (self.transport, self.subprocess.is_some()) {
            (Transport::Subprocess, false) => {
                Err(AnalyzerSpecValidationError::SubprocessConfigMissing)
            }
            (Transport::InProcess, true) => {
                Err(AnalyzerSpecValidationError::SubprocessConfigUnexpected)
            }
            _ => Ok(()),
        }
    }
}

/// Top-level shape of `<output>/.atlas/analyzers.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzersFile {
    pub schema_version: u32,
    #[serde(default)]
    pub analyzers: Vec<AnalyzerSpec>,
    /// Optional per-stage configuration the registry passes through
    /// to analysers. Open-ended so the file can carry tuning knobs
    /// without churning the schema. Keyed by analyser id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, serde_yaml::Value>,
}

impl Default for AnalyzersFile {
    fn default() -> Self {
        AnalyzersFile {
            schema_version: ANALYZERS_SCHEMA_VERSION,
            analyzers: Vec::new(),
            config: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_in_process_spec() -> AnalyzerSpec {
        AnalyzerSpec {
            id: "cargo-toml-classifier".into(),
            stage: Stage::L3,
            applicability: ApplicabilityPredicate {
                file_globs: vec!["**/Cargo.toml".into()],
                ..Default::default()
            },
            cost_class: CostClass::DeterministicCheap,
            confidence: None,
            transport: Transport::InProcess,
            subprocess: None,
            version: "1.0.3".into(),
        }
    }

    fn sample_subprocess_spec() -> AnalyzerSpec {
        AnalyzerSpec {
            id: "rust-analyzer-surface".into(),
            stage: Stage::L5,
            applicability: ApplicabilityPredicate {
                languages: vec!["rust".into()],
                ..Default::default()
            },
            cost_class: CostClass::DeterministicExpensive,
            confidence: Some(Confidence::Binary),
            transport: Transport::Subprocess,
            subprocess: Some(SubprocessConfig {
                command: vec!["rust-analyzer-surface".into(), "--stage=L5".into()],
                timeout_seconds: Some(60),
                binary_sha: None,
            }),
            version: "0.4.1".into(),
        }
    }

    fn sample_llm_fallback_spec() -> AnalyzerSpec {
        AnalyzerSpec {
            id: "llm-classify-fallback".into(),
            stage: Stage::L3,
            applicability: ApplicabilityPredicate {
                always: true,
                ..Default::default()
            },
            cost_class: CostClass::LlmCheap,
            confidence: Some(Confidence::Declines),
            transport: Transport::InProcess,
            subprocess: None,
            version: "1.0.0".into(),
        }
    }

    fn sample_analyzers_file() -> AnalyzersFile {
        AnalyzersFile {
            schema_version: ANALYZERS_SCHEMA_VERSION,
            analyzers: vec![
                sample_in_process_spec(),
                sample_subprocess_spec(),
                sample_llm_fallback_spec(),
            ],
            config: BTreeMap::new(),
        }
    }

    #[test]
    fn analyzer_spec_round_trips_through_yaml() {
        for original in [
            sample_in_process_spec(),
            sample_subprocess_spec(),
            sample_llm_fallback_spec(),
        ] {
            let yaml = serde_yaml::to_string(&original).unwrap();
            let parsed: AnalyzerSpec = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, original);
        }
    }

    #[test]
    fn analyzers_file_round_trips_through_yaml() {
        let original = sample_analyzers_file();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: AnalyzersFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn stage_variants_serialise_kebab_case() {
        for (stage, expected) in [
            (Stage::L1, "l1"),
            (Stage::L2, "l2"),
            (Stage::L3, "l3"),
            (Stage::L4, "l4"),
            (Stage::L5, "l5"),
            (Stage::L6, "l6"),
            (Stage::L7, "l7"),
            (Stage::L8, "l8"),
            (Stage::L9, "l9"),
        ] {
            let yaml = serde_yaml::to_string(&stage).unwrap();
            assert_eq!(yaml.trim(), expected);
            let parsed: Stage = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, stage);
        }
    }

    #[test]
    fn cost_class_variants_serialise_kebab_case() {
        for (cls, expected) in [
            (CostClass::DeterministicCheap, "deterministic-cheap"),
            (CostClass::DeterministicExpensive, "deterministic-expensive"),
            (CostClass::LlmCheap, "llm-cheap"),
            (CostClass::LlmExpensive, "llm-expensive"),
        ] {
            let yaml = serde_yaml::to_string(&cls).unwrap();
            assert_eq!(yaml.trim(), expected);
            let parsed: CostClass = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, cls);
        }
    }

    #[test]
    fn confidence_variants_serialise_kebab_case() {
        for (c, expected) in [
            (Confidence::Binary, "binary"),
            (Confidence::Graded, "graded"),
            (Confidence::Declines, "declines"),
        ] {
            let yaml = serde_yaml::to_string(&c).unwrap();
            assert_eq!(yaml.trim(), expected);
            let parsed: Confidence = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, c);
        }
    }

    #[test]
    fn transport_serialises_as_kebab_case_scalar() {
        for (value, expected) in [
            (Transport::InProcess, "in-process"),
            (Transport::Subprocess, "subprocess"),
        ] {
            let yaml = serde_yaml::to_string(&value).unwrap();
            assert_eq!(yaml.trim(), expected);
            let parsed: Transport = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn analyzer_spec_subprocess_emits_nested_subprocess_map() {
        // Design §6.6 requires the subprocess transport's command and
        // timeout to live under a nested `subprocess:` map, not at
        // the top level of the analyser spec.
        let spec = sample_subprocess_spec();
        let yaml = serde_yaml::to_string(&spec).unwrap();
        assert!(yaml.contains("transport: subprocess"), "got:\n{yaml}");
        assert!(yaml.contains("subprocess:"), "got:\n{yaml}");
        // command and timeout_seconds must NOT appear at AnalyzerSpec
        // top level (i.e. unindented). The nested form indents them
        // under `subprocess:`. Each top-level key starts at column 0
        // followed by ':'.
        for line in yaml.lines() {
            assert!(
                !line.starts_with("command:"),
                "`command` must be nested under subprocess:, got top-level line `{line}`"
            );
            assert!(
                !line.starts_with("timeout_seconds:"),
                "`timeout_seconds` must be nested under subprocess:, got top-level line `{line}`"
            );
        }
        let parsed: AnalyzerSpec = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn analyzer_spec_in_process_omits_subprocess_field() {
        let spec = sample_in_process_spec();
        let yaml = serde_yaml::to_string(&spec).unwrap();
        assert!(yaml.contains("transport: in-process"), "got:\n{yaml}");
        assert!(
            !yaml.contains("subprocess"),
            "in-process spec must not emit subprocess:, got:\n{yaml}"
        );
    }

    #[test]
    fn analyzer_spec_loads_design_section_6_6_rust_analyzer_surface() {
        // Verbatim subset of the design §6.6 example for
        // `rust-analyzer-surface`. The version field is included so
        // the parsed value reaches structural equality with the
        // sample fixture.
        let yaml = r#"
id: rust-analyzer-surface
stage: l5
applicability:
  languages: [rust]
cost_class: deterministic-expensive
confidence: binary
transport: subprocess
subprocess:
  command: [rust-analyzer-surface, --stage=L5]
  timeout_seconds: 60
version: 0.4.1
"#;
        let parsed: AnalyzerSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed, sample_subprocess_spec());
        parsed.validate().unwrap();

        // Round-trip preserves the same shape.
        let reemitted = serde_yaml::to_string(&parsed).unwrap();
        let reparsed: AnalyzerSpec = serde_yaml::from_str(&reemitted).unwrap();
        assert_eq!(reparsed, parsed);
        // Pin the load-bearing field names from design §6.6 so a
        // future serde rename drifting away from the wire form trips
        // here rather than slipping through the round-trip identity.
        assert!(
            reemitted.contains("transport: subprocess"),
            "design §6.6 wire form `transport: subprocess` missing:\n{reemitted}"
        );
        for field in ["subprocess:", "command:", "timeout_seconds:"] {
            assert!(
                reemitted.contains(field),
                "design §6.6 field `{field}` missing from re-emitted YAML:\n{reemitted}"
            );
        }
    }

    #[test]
    fn analyzer_spec_validate_accepts_consistent_pairs() {
        sample_in_process_spec().validate().unwrap();
        sample_subprocess_spec().validate().unwrap();
        sample_llm_fallback_spec().validate().unwrap();
    }

    #[test]
    fn analyzer_spec_validate_rejects_subprocess_without_config() {
        let mut spec = sample_subprocess_spec();
        spec.subprocess = None;
        assert_eq!(
            spec.validate().unwrap_err(),
            AnalyzerSpecValidationError::SubprocessConfigMissing
        );
    }

    #[test]
    fn analyzer_spec_validate_rejects_in_process_with_config() {
        let mut spec = sample_in_process_spec();
        spec.subprocess = Some(SubprocessConfig {
            command: vec!["x".into()],
            timeout_seconds: None,
            binary_sha: None,
        });
        assert_eq!(
            spec.validate().unwrap_err(),
            AnalyzerSpecValidationError::SubprocessConfigUnexpected
        );
    }

    #[test]
    fn subprocess_config_binary_sha_round_trips() {
        // Phase 2 PR-2: `binary_sha` is the load-bearing field that
        // captures analyser binary identity for cache fingerprints.
        let original = SubprocessConfig {
            command: vec!["foo".into()],
            timeout_seconds: Some(30),
            binary_sha: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            ),
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(yaml.contains("binary_sha:"), "got:\n{yaml}");
        let parsed: SubprocessConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn subprocess_config_binary_sha_omitted_when_none() {
        // None -> field is skipped on serialise (the YAML form
        // matches the existing on-disk shape so PR-2 is
        // wire-form-compatible with hand-authored Phase 1 fixtures).
        let original = SubprocessConfig {
            command: vec!["foo".into()],
            timeout_seconds: None,
            binary_sha: None,
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(!yaml.contains("binary_sha"), "got:\n{yaml}");
        assert!(!yaml.contains("timeout_seconds"), "got:\n{yaml}");
        let parsed: SubprocessConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn applicability_predicate_round_trips_with_only_some_fields() {
        // Only file_globs set — other fields skipped on serialise.
        let original = ApplicabilityPredicate {
            file_globs: vec!["**/Dockerfile".into()],
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(!yaml.contains("languages"), "got:\n{yaml}");
        assert!(!yaml.contains("manifest_types"), "got:\n{yaml}");
        assert!(!yaml.contains("always"), "got:\n{yaml}");
        let parsed: ApplicabilityPredicate = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn applicability_predicate_always_round_trips() {
        let original = ApplicabilityPredicate {
            always: true,
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(yaml.contains("always: true"), "got:\n{yaml}");
        let parsed: ApplicabilityPredicate = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn analyzers_file_default_has_current_schema_version() {
        let f = AnalyzersFile::default();
        assert_eq!(f.schema_version, ANALYZERS_SCHEMA_VERSION);
        assert!(f.analyzers.is_empty());
        assert!(f.config.is_empty());
    }

    #[test]
    fn stage_total_ordering_matches_declaration_order() {
        // PR-5 adds `PartialOrd + Ord` to `Stage`; downstream
        // `BTreeSet<(Stage, _)>` uses depend on this. The total order
        // is the variant declaration order: L1 < L2 < ... < L9.
        let stages = [
            Stage::L1,
            Stage::L2,
            Stage::L3,
            Stage::L4,
            Stage::L5,
            Stage::L6,
            Stage::L7,
            Stage::L8,
            Stage::L9,
        ];
        for window in stages.windows(2) {
            assert!(window[0] < window[1], "stage ord: {:?}", window);
        }
        // Bonus: BTreeSet round-trip preserves the ordering.
        let mut set = std::collections::BTreeSet::new();
        for s in stages {
            set.insert(s);
        }
        let sorted: Vec<_> = set.into_iter().collect();
        assert_eq!(sorted, stages);
    }
}
