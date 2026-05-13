//! PR-2 schema-drift catcher (brainstorm decision row 2).
//!
//! Asserts each `build_dispatch_*_prompt` emits a string containing
//! exactly one fenced ```yaml block AND that block deserializes (via
//! `serde_yaml::from_str`) into the target struct.
//!
//! If `SubsystemsOverrideFile` / `ComponentsOverrideFile` gains or
//! renames a field, the embedded YAML example in the prompt template
//! must update in lock-step or this test fails fast. The drift catcher
//! is the answer to: "what stops the prompt's schema advertisement
//! from silently desyncing from the struct the deserializer expects?"

use std::path::Path;

use atlas_agents::runtime::dispatch::{
    build_dispatch_components_prompt, build_dispatch_subsystems_prompt, ComponentsOverrideFile,
    SubsystemPartition, SubsystemsOverrideFile,
};
use atlas_agents::runtime::prompt_examples::extract_yaml_fence;

fn synthetic_workspace_root() -> &'static Path {
    Path::new("/tmp/synthetic-workspace")
}

fn synthetic_subsystem_partition() -> SubsystemPartition {
    SubsystemPartition {
        id: "synthetic-subsystem".to_string(),
        members: vec!["foo".to_string(), "bar".to_string()],
    }
}

#[test]
fn dispatch_subsystems_prompt_yaml_example_deserializes() {
    let prompt = build_dispatch_subsystems_prompt(synthetic_workspace_root(), 15, 30);
    let yaml_body = extract_yaml_fence(&prompt)
        .expect("dispatch-subsystems prompt must contain a fenced ```yaml block");
    let parsed: SubsystemsOverrideFile = serde_yaml::from_str(yaml_body)
        .expect("embedded YAML example must deserialize into SubsystemsOverrideFile");
    assert!(
        !parsed.subsystems.is_empty(),
        "example must include at least one subsystem entry"
    );
    assert_eq!(
        parsed.schema_version, 1,
        "example must pin schema_version: 1"
    );
}

#[test]
fn dispatch_components_prompt_yaml_example_deserializes() {
    let subsystem = synthetic_subsystem_partition();
    let prompt = build_dispatch_components_prompt(synthetic_workspace_root(), &subsystem, 15, 30);
    let yaml_body = extract_yaml_fence(&prompt)
        .expect("dispatch-components prompt must contain a fenced ```yaml block");
    let parsed: ComponentsOverrideFile = serde_yaml::from_str(yaml_body)
        .expect("embedded YAML example must deserialize into ComponentsOverrideFile");
    assert_eq!(
        parsed.schema_version, 1,
        "example must pin schema_version: 1"
    );
}

#[test]
fn dispatch_subsystems_prompt_embeds_caller_supplied_caps() {
    // Caps are caller-supplied; the prompt embeds them verbatim so
    // prompt-text and AgentRequest::max_steps cannot drift (decision
    // row 4). Use non-default values so we can distinguish them from
    // any hardcoded constants that might've slipped in.
    let prompt = build_dispatch_subsystems_prompt(synthetic_workspace_root(), 7, 42);
    assert!(
        prompt.contains("soft cap 7"),
        "prompt must embed the caller-supplied soft cap value: {prompt}"
    );
    assert!(
        prompt.contains("hard cap 42"),
        "prompt must embed the caller-supplied hard cap value"
    );
}

#[test]
fn dispatch_components_prompt_embeds_caller_supplied_caps() {
    let subsystem = synthetic_subsystem_partition();
    let prompt = build_dispatch_components_prompt(synthetic_workspace_root(), &subsystem, 7, 42);
    assert!(
        prompt.contains("soft cap 7"),
        "prompt must embed the caller-supplied soft cap value"
    );
    assert!(
        prompt.contains("hard cap 42"),
        "prompt must embed the caller-supplied hard cap value"
    );
}

#[test]
fn dispatch_subsystems_prompt_references_evidence_floor_rubric() {
    // Decision row 5: the four-grade rubric must be advertised so the
    // LLM emits self-grades that map onto the Lane A evidence ladder.
    let prompt = build_dispatch_subsystems_prompt(synthetic_workspace_root(), 15, 30);
    for grade in ["strong", "moderate", "weak", "declines"] {
        assert!(
            prompt.contains(grade),
            "prompt must reference grade `{grade}` in the rubric"
        );
    }
}

#[test]
fn dispatch_subsystems_prompt_keys_on_test_backend_substring() {
    // Backend test stubs (DispatchStagedBackend in tests/dispatch_shortcircuit.rs)
    // key canned responses on the substring "dispatch subsystems". The
    // production prompt must preserve that exact phrase or the tests
    // will silently miss.
    let prompt = build_dispatch_subsystems_prompt(synthetic_workspace_root(), 15, 30);
    assert!(
        prompt.to_lowercase().contains("dispatch subsystems"),
        "prompt must contain the `dispatch subsystems` substring so the test backend keys match"
    );
}

#[test]
fn dispatch_components_prompt_keys_on_test_backend_substring() {
    let subsystem = synthetic_subsystem_partition();
    let prompt = build_dispatch_components_prompt(synthetic_workspace_root(), &subsystem, 15, 30);
    assert!(
        prompt.to_lowercase().contains("dispatch components"),
        "prompt must contain the `dispatch components` substring so the test backend keys match"
    );
}
