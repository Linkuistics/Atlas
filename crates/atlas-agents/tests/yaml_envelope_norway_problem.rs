//! PR-2 YAML-specific failure-mode regression test
//! (brainstorm §12.8 Risk 1 mitigation (c)).
//!
//! Catches accidental removal of the [`deserialize_string_strict`]
//! adapter from any guarded field — and pins the YAML-1.2 behaviour
//! that `NO` / `yes` / `on` naturally remain strings (the historic
//! Norway problem was a YAML-1.1-era hazard; `serde_yaml 0.9`
//! follows YAML 1.2). The active hazards for the LLM-spine path
//! today are:
//!
//! - `id: true` / `id: false` — YAML 1.2 booleans.
//! - `id: 1.10` — YAML 1.2 implicit numeric (loses trailing zero).
//! - `id: 123` — YAML 1.2 integer.
//! - `id: null` / `id: ~` — YAML 1.2 null.
//!
//! The strict adapter rejects each with an actionable error so Lane
//! A's retry loop can prompt the LLM to quote the value.
//!
//! [`deserialize_string_strict`]: atlas_agents::runtime::yaml_strict::deserialize_string_strict

use atlas_agents::runtime::dispatch::SubsystemsOverrideFile;

#[test]
fn yaml_1_2_keeps_no_as_string_naturally() {
    // YAML 1.2 reads `NO` as a string — no Norway-problem coercion
    // applies, the adapter accepts.
    let yaml = "schema_version: 1\nsubsystems:\n  - id: NO\n    members: []\n";
    let parsed: SubsystemsOverrideFile = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.subsystems[0].id, "NO");
}

#[test]
fn yaml_1_2_keeps_yes_as_string_naturally() {
    let yaml = "schema_version: 1\nsubsystems:\n  - id: yes\n    members: []\n";
    let parsed: SubsystemsOverrideFile = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.subsystems[0].id, "yes");
}

#[test]
fn yaml_1_2_keeps_on_as_string_naturally() {
    let yaml = "schema_version: 1\nsubsystems:\n  - id: on\n    members: []\n";
    let parsed: SubsystemsOverrideFile = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.subsystems[0].id, "on");
}

#[test]
fn unquoted_true_in_subsystem_id_is_rejected() {
    // Lowercase `true` IS a YAML 1.2 bool. The strict adapter rejects
    // and the error names the failure mode so Lane A's retry prompt
    // can ask the LLM to quote.
    let yaml = "schema_version: 1\nsubsystems:\n  - id: true\n    members: []\n";
    let err = serde_yaml::from_str::<SubsystemsOverrideFile>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("Norway-problem"),
        "error must name the failure mode for actionable LLM feedback; got: {err}"
    );
}

#[test]
fn unquoted_false_in_subsystem_id_is_rejected() {
    let yaml = "schema_version: 1\nsubsystems:\n  - id: false\n    members: []\n";
    let err = serde_yaml::from_str::<SubsystemsOverrideFile>(yaml).unwrap_err();
    assert!(err.to_string().contains("Norway-problem"));
}

#[test]
fn unquoted_version_shaped_number_in_subsystem_id_is_rejected() {
    // `1.10` is a YAML float that silently loses the trailing zero.
    // The adapter rejects so the LLM emits a quoted "1.10".
    let yaml = "schema_version: 1\nsubsystems:\n  - id: 1.10\n    members: []\n";
    let err = serde_yaml::from_str::<SubsystemsOverrideFile>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("implicit numeric"),
        "error must name the failure mode for actionable LLM feedback; got: {err}"
    );
}

#[test]
fn unquoted_integer_in_subsystem_id_is_rejected() {
    let yaml = "schema_version: 1\nsubsystems:\n  - id: 42\n    members: []\n";
    let err = serde_yaml::from_str::<SubsystemsOverrideFile>(yaml).unwrap_err();
    assert!(err.to_string().contains("implicit numeric"));
}

#[test]
fn unquoted_null_in_subsystem_id_is_rejected() {
    let yaml = "schema_version: 1\nsubsystems:\n  - id: null\n    members: []\n";
    let err = serde_yaml::from_str::<SubsystemsOverrideFile>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("YAML null"),
        "error must name the failure mode for actionable LLM feedback; got: {err}"
    );
}

#[test]
fn quoted_identity_shaped_scalars_round_trip_unchanged() {
    // Once quoted, every implicit-typing hazard is unambiguously a
    // string. This is the canonical authoring shape the dispatch
    // prompts ask the LLM to emit.
    let yaml = r#"schema_version: 1
subsystems:
  - id: "true"
    members: []
  - id: "1.10"
    members: []
  - id: "42"
    members: []
"#;
    let parsed: SubsystemsOverrideFile = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.subsystems.len(), 3);
    assert_eq!(parsed.subsystems[0].id, "true");
    assert_eq!(parsed.subsystems[1].id, "1.10");
    assert_eq!(parsed.subsystems[2].id, "42");
}
