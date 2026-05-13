//! Typed LLM-agent output shapes for the four non-dispatch stages.
//!
//! Each struct is `Deserialize`-from-YAML — the production prompts in
//! [`crate::runtime::mod`] embed a fenced ```yaml example whose body
//! deserializes back into the target struct via `serde_yaml::from_str`.
//! The schema-drift catcher tests in
//! `crates/atlas-agents/tests/{classify,reduce,project}_prompt_shape.rs`
//! round-trip each embedded example to keep prompt-text and struct-shape
//! in lock-step.
//!
//! Every struct carries an `evidence_pointers: Vec<EvidencePointer>`
//! field. Downstream LLM consumers (framing #2 from the brainstorm —
//! Atlas exists to feed other LLM tools with monorepo context) verify
//! analyses by re-reading cited evidence; the field is non-optional and
//! Lane A's per-stage evidence scoring (PR-3
//! [`crate::runtime::audit::evidence`]) clamps the LLM's self-grade
//! against the transcript-grounded ceiling.
//!
//! Identity-shaped string fields use the strict-string adapter
//! (`yaml_strict::deserialize_string_strict`) so YAML implicit-typing
//! coercion (`component_id: NO` → bool, `1.10` → float) surfaces as a
//! Lane A retry rather than a silent malformed analysis.
//!
//! `Lifecycle` re-uses `component_ontology::LifecycleScope` (closed
//! kebab-case vocabulary). `ComponentKind` and `Language` are open
//! vocabularies — strict-string newtypes today, refactor candidates if
//! a closed enum lands later.

use crate::events::Grade;
use crate::runtime::yaml_strict::deserialize_string_strict;
use component_ontology::LifecycleScope as Lifecycle;
use serde::{Deserialize, Deserializer, Serialize};

/// Strict-string newtype wrapping a component id reference. Used in
/// `Vec<ComponentIdRef>` positions where a plain `Vec<String>` would
/// bypass the strict-string adapter — serde's `Vec` deserializer
/// re-uses the inner type's `Deserialize`, so the per-element strict
/// check lands here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComponentIdRef(#[serde(deserialize_with = "deserialize_string_strict")] pub String);

impl ComponentIdRef {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Strict-string newtype for component kind. Open vocabulary today
/// (the deterministic engine carries `kind: String` for the same
/// reason — see `atlas_index::ComponentEntry::kind`). The strict
/// adapter defends against `kind: 0123` accidentally parsing as a
/// number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComponentKind(#[serde(deserialize_with = "deserialize_string_strict")] pub String);

impl ComponentKind {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Strict-string newtype for language. Open vocabulary today.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Language(#[serde(deserialize_with = "deserialize_string_strict")] pub String);

impl Language {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Pointer into the workspace identifying the evidence the LLM cites.
/// `path` is workspace-relative; `line_range` is `(start, end)` inclusive
/// when present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidencePointer {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<(u32, u32)>,
}

/// Deserialize a `Grade` from a case-insensitive lowercase string. The
/// production prompts advertise grades as `"strong" / "moderate" /
/// "weak" / "declines"` (matching the existing dispatch shape); without
/// this adapter `serde` would reject lowercase forms because `Grade`'s
/// default PascalCase serialisation expects `"Strong"`, etc.
///
/// Lives at the `confidence_grade` field of every PR-3 output struct.
fn deserialize_grade_lowercase<'de, D>(deserializer: D) -> Result<Grade, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.to_lowercase().as_str() {
        "strong" => Ok(Grade::Strong),
        "moderate" => Ok(Grade::Moderate),
        "weak" => Ok(Grade::Weak),
        "declines" => Ok(Grade::Declines),
        other => Err(serde::de::Error::custom(format!(
            "expected confidence_grade in {{strong, moderate, weak, declines}}, got `{other}`"
        ))),
    }
}

/// Output of the per-component Classify stage.
///
/// Produced by `build_classify_prompt`. The reduce stage consumes one
/// `ClassifyAgentOutput` per component; the canonical-schema shim
/// projects each into a row of the canonical `components` artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifyAgentOutput {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub component_id: String,
    pub kind: ComponentKind,
    pub language: Language,
    pub lifecycle: Lifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem_hint: Option<String>,
    pub evidence_pointers: Vec<EvidencePointer>,
    #[serde(deserialize_with = "deserialize_grade_lowercase")]
    pub confidence_grade: Grade,
}

/// Reference to a contract surfaced by a subsystem. `kind` is
/// domain-specific (e.g. `"http-api"`, `"rust-trait"`); free-text for
/// now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRef {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub id: String,
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<EvidencePointer>,
}

/// Edge between two components or subsystems. `kind` mirrors the
/// component-ontology edge vocabulary (`depends-on`, `calls`,
/// `provides-contract`, ...). Open here — the canonical shim validates
/// against `component_ontology::EdgeKind` if a stricter check is wanted
/// in a later refactor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRef {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub from: String,
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub to: String,
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub kind: String,
}

/// Classification of a refactoring opportunity the reducer surfaced.
/// Framing #2 use-case (b) — Atlas exists in part to surface
/// refactoring cues for downstream LLM tools — so this field is
/// load-bearing on `ReduceAgentOutput`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefactoringCueKind {
    Duplication,
    MisModularised,
    AbstractionOpportunity,
    DependencyInversion,
    DeadCode,
    Other,
}

/// One refactoring cue. `rationale` is a 1-sentence summary suitable
/// for a downstream LLM tool to include in a code-review comment or a
/// refactor proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefactoringCue {
    pub kind: RefactoringCueKind,
    pub component_ids: Vec<ComponentIdRef>,
    pub rationale: String,
    pub evidence_pointers: Vec<EvidencePointer>,
}

/// Output of the per-subsystem Reduce stage.
///
/// Produced by `build_reduce_prompt`. The project stage consumes one
/// `ReduceAgentOutput` per subsystem; the canonical-schema shim
/// projects each into a row of the canonical `subsystems` artifact.
///
/// `declared_child_component_ids` echoes back the per-subsystem child
/// list the runtime handed to the reducer (per the prompt rubric).
/// Lane A's reduce-stage evidence scorer reads this against
/// `component_ids` (the children the reducer actually accounted for)
/// to compute the coverage ratio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReduceAgentOutput {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub subsystem_id: String,
    pub purpose: String,
    pub declared_child_component_ids: Vec<ComponentIdRef>,
    pub component_ids: Vec<ComponentIdRef>,
    #[serde(default)]
    pub key_contracts: Vec<ContractRef>,
    #[serde(default)]
    pub internal_edges: Vec<EdgeRef>,
    #[serde(default)]
    pub refactoring_cues: Vec<RefactoringCue>,
    pub evidence_pointers: Vec<EvidencePointer>,
    #[serde(deserialize_with = "deserialize_grade_lowercase")]
    pub confidence_grade: Grade,
}

/// One row of the workspace-level subsystem catalog the project stage
/// emits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemSummary {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub subsystem_id: String,
    pub purpose: String,
    pub component_count: u32,
}

/// Recursive doc-scaffold node. Each section is a heading + a list of
/// evidence references downstream doc-generation tools can use to fill
/// the section body. Children represent subheadings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocSection {
    pub heading: String,
    #[serde(default)]
    pub source_references: Vec<EvidencePointer>,
    #[serde(default)]
    pub child_sections: Vec<DocSection>,
}

/// Top-level doc scaffold. Framing #2 use case (c) — documentation
/// generation — depends on this shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocScaffoldOutline {
    #[serde(default)]
    pub sections: Vec<DocSection>,
}

/// Output of the workspace-level Project stage.
///
/// Produced by `build_project_prompt`. Consumers (downstream LLM tools,
/// framing #2) read this first for a high-level architecture summary,
/// then drill into per-subsystem reduces and per-component classifies
/// as needed.
///
/// `declared_subsystem_ids` echoes back the subsystem-id list the
/// runtime handed to the project agent. Lane A's project-stage
/// evidence scorer reads this against `subsystem_catalog` (the
/// subsystems the agent actually cataloged) to compute coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAgentOutput {
    pub workspace_purpose: String,
    pub declared_subsystem_ids: Vec<ComponentIdRef>,
    pub subsystem_catalog: Vec<SubsystemSummary>,
    #[serde(default)]
    pub cross_subsystem_edges: Vec<EdgeRef>,
    #[serde(default)]
    pub workspace_refactoring_cues: Vec<RefactoringCue>,
    pub doc_scaffold: DocScaffoldOutline,
    #[serde(deserialize_with = "deserialize_grade_lowercase")]
    pub confidence_grade: Grade,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_output_round_trips_via_yaml() {
        let raw = r#"
component_id: "atlas-cli"
kind: "rust-binary"
language: "rust"
lifecycle: "build"
subsystem_hint: "cli"
evidence_pointers:
  - path: "crates/atlas-cli/Cargo.toml"
    line_range: [1, 30]
  - path: "crates/atlas-cli/src/main.rs"
confidence_grade: "strong"
"#;
        let parsed: ClassifyAgentOutput = serde_yaml::from_str(raw).unwrap();
        assert_eq!(parsed.component_id, "atlas-cli");
        assert_eq!(parsed.kind.as_str(), "rust-binary");
        assert_eq!(parsed.language.as_str(), "rust");
        assert_eq!(parsed.lifecycle, Lifecycle::Build);
        assert_eq!(parsed.subsystem_hint.as_deref(), Some("cli"));
        assert_eq!(parsed.evidence_pointers.len(), 2);
        assert_eq!(parsed.confidence_grade, Grade::Strong);

        let re_yaml = serde_yaml::to_string(&parsed).unwrap();
        let re_parsed: ClassifyAgentOutput = serde_yaml::from_str(&re_yaml).unwrap();
        assert_eq!(re_parsed.component_id, parsed.component_id);
        assert_eq!(re_parsed.lifecycle, parsed.lifecycle);
    }

    #[test]
    fn reduce_output_round_trips_with_refactoring_cues() {
        let raw = r#"
subsystem_id: "agents"
purpose: "Async LLM-spine runtime owning the per-stage tool loop and Lane A/B audits."
declared_child_component_ids:
  - "atlas-agents"
  - "atlas-llm"
component_ids:
  - "atlas-agents"
  - "atlas-llm"
key_contracts:
  - id: "tools/parse_cargo_toml"
    kind: "tool-handle"
    source_path:
      path: "crates/atlas-agents/src/tools/parse_cargo_toml.rs"
internal_edges:
  - from: "atlas-agents"
    to: "atlas-llm"
    kind: "depends-on"
refactoring_cues:
  - kind: "abstraction-opportunity"
    component_ids: ["atlas-agents"]
    rationale: "Tool catalog could be split per stage."
    evidence_pointers:
      - path: "crates/atlas-agents/src/tools/mod.rs"
evidence_pointers:
  - path: "crates/atlas-agents/Cargo.toml"
confidence_grade: "moderate"
"#;
        let parsed: ReduceAgentOutput = serde_yaml::from_str(raw).unwrap();
        assert_eq!(parsed.subsystem_id, "agents");
        assert_eq!(parsed.component_ids.len(), 2);
        assert_eq!(parsed.refactoring_cues.len(), 1);
        assert_eq!(
            parsed.refactoring_cues[0].kind,
            RefactoringCueKind::AbstractionOpportunity
        );
        assert_eq!(parsed.confidence_grade, Grade::Moderate);
    }

    #[test]
    fn project_output_round_trips_with_doc_scaffold() {
        let raw = r#"
workspace_purpose: "Atlas: LLM-spine monorepo analysis tool feeding downstream LLM consumers."
declared_subsystem_ids:
  - "agents"
  - "cli"
subsystem_catalog:
  - subsystem_id: "agents"
    purpose: "LLM-spine runtime."
    component_count: 3
  - subsystem_id: "cli"
    purpose: "CLI entry points."
    component_count: 1
cross_subsystem_edges:
  - from: "cli"
    to: "agents"
    kind: "depends-on"
workspace_refactoring_cues: []
doc_scaffold:
  sections:
    - heading: "Architecture"
      source_references:
        - path: "docs/architecture.md"
      child_sections:
        - heading: "Agent runtime"
          source_references:
            - path: "crates/atlas-agents/src/runtime/mod.rs"
confidence_grade: "weak"
"#;
        let parsed: ProjectAgentOutput = serde_yaml::from_str(raw).unwrap();
        assert_eq!(parsed.subsystem_catalog.len(), 2);
        assert_eq!(parsed.doc_scaffold.sections.len(), 1);
        assert_eq!(parsed.doc_scaffold.sections[0].child_sections.len(), 1);
        assert_eq!(parsed.confidence_grade, Grade::Weak);
    }

    #[test]
    fn evidence_pointer_optional_line_range_omits_on_serialize() {
        let ep = EvidencePointer {
            path: "a/b.rs".to_string(),
            line_range: None,
        };
        let yaml = serde_yaml::to_string(&ep).unwrap();
        assert!(!yaml.contains("line_range"), "got:\n{yaml}");
    }

    #[test]
    fn component_id_ref_rejects_yaml_implicit_bool() {
        // `component_ids: [true]` would silently coerce a YAML bool
        // into the string-position. The strict adapter rejects so
        // Lane A retries can ask the LLM to quote identity-shaped
        // scalars.
        let err = serde_yaml::from_str::<Vec<ComponentIdRef>>("- true\n").unwrap_err();
        assert!(err.to_string().contains("Norway-problem"), "got: {err}");
    }

    #[test]
    fn component_kind_accepts_kebab_case_string() {
        let k: ComponentKind = serde_yaml::from_str("\"rust-library\"").unwrap();
        assert_eq!(k.as_str(), "rust-library");
    }

    #[test]
    fn confidence_grade_accepts_lowercase_and_titlecase() {
        // Lower-case matches the rubric the prompts advertise.
        let raw = r#"
component_id: "x"
kind: "lib"
language: "rust"
lifecycle: "build"
evidence_pointers: []
confidence_grade: "strong"
"#;
        let parsed: ClassifyAgentOutput = serde_yaml::from_str(raw).unwrap();
        assert_eq!(parsed.confidence_grade, Grade::Strong);

        // Title-case also works (defensive — some upstream models capitalise).
        let raw_title = raw.replace("strong", "Strong");
        let parsed_title: ClassifyAgentOutput = serde_yaml::from_str(&raw_title).unwrap();
        assert_eq!(parsed_title.confidence_grade, Grade::Strong);
    }

    #[test]
    fn confidence_grade_rejects_unknown_value() {
        let raw = r#"
component_id: "x"
kind: "lib"
language: "rust"
lifecycle: "build"
evidence_pointers: []
confidence_grade: "uncertain"
"#;
        let err = serde_yaml::from_str::<ClassifyAgentOutput>(raw).unwrap_err();
        assert!(
            err.to_string().contains("confidence_grade"),
            "error must mention the field: {err}"
        );
    }
}
