//! PR-3 schema-drift catcher for `build_classify_prompt` (decision row
//! 2 generalised to the non-dispatch stages).
//!
//! Asserts the production classify prompt:
//!   - contains exactly one fenced ```yaml block,
//!   - that block deserializes into [`ClassifyAgentOutput`],
//!   - embeds the caller-supplied soft/hard caps verbatim,
//!   - references the four-grade rubric so the LLM emits grades that
//!     map onto Lane A's evidence ladder.
//!
//! If `ClassifyAgentOutput` gains or renames a field, the embedded YAML
//! example in the prompt template must update in lock-step or this
//! test fails fast.

use std::path::Path;

use atlas_agents::runtime::build_classify_prompt;
use atlas_agents::runtime::dispatch::{ComponentFieldOverrides, ComponentPartition};
use atlas_agents::runtime::outputs::ClassifyAgentOutput;
use atlas_agents::runtime::prompt_examples::extract_yaml_fence;

fn synthetic_workspace_root() -> &'static Path {
    Path::new("/tmp/synthetic-workspace")
}

fn synthetic_component() -> ComponentPartition {
    ComponentPartition {
        id: "atlas-cli".to_string(),
        subsystem_id: "cli".to_string(),
        field_overrides: ComponentFieldOverrides::default(),
    }
}

#[test]
fn classify_prompt_yaml_example_deserializes() {
    let prompt = build_classify_prompt(synthetic_workspace_root(), &synthetic_component(), 6, 12);
    let yaml_body =
        extract_yaml_fence(&prompt).expect("classify prompt must contain a fenced ```yaml block");
    let parsed: ClassifyAgentOutput = serde_yaml::from_str(yaml_body)
        .expect("embedded YAML example must deserialize into ClassifyAgentOutput");
    assert!(
        !parsed.evidence_pointers.is_empty(),
        "ClassifyAgentOutput example must include evidence_pointers (framing #2 — \
         downstream consumers verify analyses by re-reading cited evidence)"
    );
}

#[test]
fn classify_prompt_embeds_caller_supplied_caps() {
    // Caps are caller-supplied; the prompt embeds them verbatim so
    // prompt-text and AgentRequest::max_steps cannot drift. Use
    // non-default values so we can distinguish them from any hardcoded
    // constants that might've slipped in.
    let prompt = build_classify_prompt(synthetic_workspace_root(), &synthetic_component(), 3, 17);
    assert!(
        prompt.contains("soft cap 3"),
        "prompt must embed the caller-supplied soft cap value: {prompt}"
    );
    assert!(
        prompt.contains("hard cap 17"),
        "prompt must embed the caller-supplied hard cap value"
    );
}

#[test]
fn classify_prompt_references_four_grade_rubric() {
    let prompt = build_classify_prompt(synthetic_workspace_root(), &synthetic_component(), 6, 12);
    for grade in ["strong", "moderate", "weak", "declines"] {
        assert!(
            prompt.contains(grade),
            "prompt must reference grade `{grade}` in the rubric"
        );
    }
}

#[test]
fn classify_prompt_keys_on_test_backend_substring() {
    // Backend test stubs that key canned responses on the substring
    // "classify" must continue to match this production prompt.
    let prompt = build_classify_prompt(synthetic_workspace_root(), &synthetic_component(), 6, 12);
    assert!(
        prompt.to_lowercase().contains("classify"),
        "prompt must contain the `classify` substring so backend test stubs key correctly"
    );
}

#[test]
fn classify_prompt_embeds_component_id_in_example() {
    // The prompt is per-component — its embedded example MUST quote
    // the target component id so the LLM doesn't echo a stale literal.
    let component = ComponentPartition {
        id: "unusual-component-id".to_string(),
        subsystem_id: "x".to_string(),
        field_overrides: ComponentFieldOverrides::default(),
    };
    let prompt = build_classify_prompt(synthetic_workspace_root(), &component, 6, 12);
    assert!(
        prompt.contains("unusual-component-id"),
        "prompt must embed the target component id"
    );
}
