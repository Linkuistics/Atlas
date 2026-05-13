//! PR-3 schema-drift catcher for `build_reduce_prompt`.

use std::path::Path;

use atlas_agents::runtime::build_reduce_prompt;
use atlas_agents::runtime::dispatch::SubsystemPartition;
use atlas_agents::runtime::outputs::ReduceAgentOutput;
use atlas_agents::runtime::prompt_examples::extract_yaml_fence;

fn synthetic_workspace_root() -> &'static Path {
    Path::new("/tmp/synthetic-workspace")
}

fn synthetic_subsystem() -> SubsystemPartition {
    SubsystemPartition {
        id: "agents".to_string(),
        members: vec!["atlas-agents".to_string(), "atlas-llm".to_string()],
    }
}

fn synthetic_classify_rollup() -> Vec<(String, String, String)> {
    vec![
        (
            "atlas-agents".to_string(),
            "rust-library".to_string(),
            "rust".to_string(),
        ),
        (
            "atlas-llm".to_string(),
            "rust-library".to_string(),
            "rust".to_string(),
        ),
    ]
}

#[test]
fn reduce_prompt_yaml_example_deserializes() {
    let prompt = build_reduce_prompt(
        synthetic_workspace_root(),
        &synthetic_subsystem(),
        &synthetic_classify_rollup(),
        4,
        8,
    );
    let yaml_body =
        extract_yaml_fence(&prompt).expect("reduce prompt must contain a fenced ```yaml block");
    let parsed: ReduceAgentOutput = serde_yaml::from_str(yaml_body)
        .expect("embedded YAML example must deserialize into ReduceAgentOutput");
    assert!(
        !parsed.declared_child_component_ids.is_empty(),
        "ReduceAgentOutput example must populate declared_child_component_ids \
         (the denominator of Lane A's reduce-stage evidence ratio)"
    );
    assert!(
        !parsed.evidence_pointers.is_empty(),
        "ReduceAgentOutput example must include evidence_pointers"
    );
}

#[test]
fn reduce_prompt_embeds_caller_supplied_caps() {
    let prompt = build_reduce_prompt(
        synthetic_workspace_root(),
        &synthetic_subsystem(),
        &synthetic_classify_rollup(),
        3,
        17,
    );
    assert!(
        prompt.contains("soft cap 3"),
        "prompt must embed the caller-supplied soft cap"
    );
    assert!(
        prompt.contains("hard cap 17"),
        "prompt must embed the caller-supplied hard cap"
    );
}

#[test]
fn reduce_prompt_references_four_grade_rubric() {
    let prompt = build_reduce_prompt(
        synthetic_workspace_root(),
        &synthetic_subsystem(),
        &synthetic_classify_rollup(),
        4,
        8,
    );
    for grade in ["strong", "moderate", "weak", "declines"] {
        assert!(
            prompt.contains(grade),
            "prompt must reference grade `{grade}` in the rubric"
        );
    }
}

#[test]
fn reduce_prompt_embeds_classify_rollup() {
    // The reducer must see the per-component classify outputs in its
    // prompt. The rollup shape is `(id, kind, language)` per component;
    // the prompt embeds them in a human-readable list.
    let prompt = build_reduce_prompt(
        synthetic_workspace_root(),
        &synthetic_subsystem(),
        &synthetic_classify_rollup(),
        4,
        8,
    );
    for id in ["atlas-agents", "atlas-llm"] {
        assert!(
            prompt.contains(id),
            "prompt must embed classify-rollup component id `{id}`"
        );
    }
}

#[test]
fn reduce_prompt_keys_on_test_backend_substring() {
    let prompt = build_reduce_prompt(
        synthetic_workspace_root(),
        &synthetic_subsystem(),
        &synthetic_classify_rollup(),
        4,
        8,
    );
    assert!(
        prompt.to_lowercase().contains("reduce"),
        "prompt must contain the `reduce` substring"
    );
}

#[test]
fn reduce_prompt_references_refactoring_cue_vocabulary() {
    // Framing #2 use-case (b) — refactoring cues are load-bearing.
    // The prompt rubric must surface the cue kinds so the LLM picks
    // from the closed vocabulary.
    let prompt = build_reduce_prompt(
        synthetic_workspace_root(),
        &synthetic_subsystem(),
        &synthetic_classify_rollup(),
        4,
        8,
    );
    for cue_kind in ["duplication", "abstraction-opportunity"] {
        assert!(
            prompt.contains(cue_kind),
            "prompt must reference refactoring-cue kind `{cue_kind}`"
        );
    }
}
