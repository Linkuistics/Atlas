//! PR-3 schema-drift catcher for `build_project_prompt` (new — no
//! PR-7 placeholder predecessor).

use std::path::Path;

use atlas_agents::runtime::build_project_prompt;
use atlas_agents::runtime::outputs::ProjectAgentOutput;
use atlas_agents::runtime::prompt_examples::extract_yaml_fence;

fn synthetic_workspace_root() -> &'static Path {
    Path::new("/tmp/synthetic-workspace")
}

fn synthetic_reduce_rollup() -> Vec<(String, String, u32)> {
    vec![
        (
            "agents".to_string(),
            "Async LLM-spine runtime owning the per-stage tool loop.".to_string(),
            3,
        ),
        (
            "cli".to_string(),
            "CLI entry points and pipeline orchestration.".to_string(),
            1,
        ),
    ]
}

#[test]
fn project_prompt_yaml_example_deserializes() {
    let prompt = build_project_prompt(synthetic_workspace_root(), &synthetic_reduce_rollup(), 4, 8);
    let yaml_body =
        extract_yaml_fence(&prompt).expect("project prompt must contain a fenced ```yaml block");
    let parsed: ProjectAgentOutput = serde_yaml::from_str(yaml_body)
        .expect("embedded YAML example must deserialize into ProjectAgentOutput");
    assert!(
        !parsed.declared_subsystem_ids.is_empty(),
        "ProjectAgentOutput example must populate declared_subsystem_ids"
    );
    assert!(
        !parsed.doc_scaffold.sections.is_empty(),
        "ProjectAgentOutput example must populate doc_scaffold.sections \
         (framing #2 use-case (c) — documentation generation)"
    );
}

#[test]
fn project_prompt_embeds_caller_supplied_caps() {
    let prompt = build_project_prompt(
        synthetic_workspace_root(),
        &synthetic_reduce_rollup(),
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
fn project_prompt_references_four_grade_rubric() {
    let prompt = build_project_prompt(synthetic_workspace_root(), &synthetic_reduce_rollup(), 4, 8);
    for grade in ["strong", "moderate", "weak", "declines"] {
        assert!(
            prompt.contains(grade),
            "prompt must reference grade `{grade}` in the rubric"
        );
    }
}

#[test]
fn project_prompt_embeds_reduce_rollup() {
    // The project agent must see the per-subsystem reduce outputs in
    // its prompt.
    let prompt = build_project_prompt(synthetic_workspace_root(), &synthetic_reduce_rollup(), 4, 8);
    for id in ["agents", "cli"] {
        assert!(
            prompt.contains(id),
            "prompt must embed reduce-rollup subsystem id `{id}`"
        );
    }
}

#[test]
fn project_prompt_keys_on_test_backend_substring() {
    let prompt = build_project_prompt(synthetic_workspace_root(), &synthetic_reduce_rollup(), 4, 8);
    assert!(
        prompt.to_lowercase().contains("project"),
        "prompt must contain the `project` substring"
    );
}

#[test]
fn project_prompt_references_doc_scaffold_loadbearing() {
    let prompt = build_project_prompt(synthetic_workspace_root(), &synthetic_reduce_rollup(), 4, 8);
    assert!(
        prompt.contains("doc_scaffold"),
        "prompt must reference doc_scaffold (framing #2 use-case (c))"
    );
}
