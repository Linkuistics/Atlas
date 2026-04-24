//! Per-component interaction-surface record, ported from Ravel-Lite's
//! Stage 1 schema (see `Ravel-Lite/src/discover/schema.rs`).
//!
//! The shape follows Ravel-Lite's `SurfaceRecord` exactly, with
//! `explicit_cross_project_mentions` renamed to
//! `explicit_cross_component_mentions` — Atlas's unit of analysis is a
//! component, not a project. The closed vocabulary
//! [`InteractionRoleHint`] is likewise a verbatim port: adding a variant
//! to one side without the other fails the drift test in `lib.rs`.

use serde::{Deserialize, Serialize};

/// Closed vocabulary of advisory labels a component's own prose may
/// declare about its interaction role. L5 emits these from README /
/// top-level docs; L6 treats them as priors, not verdicts. Unknown
/// values are rejected at deserialisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InteractionRoleHint {
    Generator,
    Orchestrator,
    TestHarness,
    SpecDocument,
    Spawner,
    DocumentedBy,
    Client,
    Server,
    Library,
    Tool,
}

impl InteractionRoleHint {
    pub fn as_str(self) -> &'static str {
        match self {
            InteractionRoleHint::Generator => "generator",
            InteractionRoleHint::Orchestrator => "orchestrator",
            InteractionRoleHint::TestHarness => "test-harness",
            InteractionRoleHint::SpecDocument => "spec-document",
            InteractionRoleHint::Spawner => "spawner",
            InteractionRoleHint::DocumentedBy => "documented-by",
            InteractionRoleHint::Client => "client",
            InteractionRoleHint::Server => "server",
            InteractionRoleHint::Library => "library",
            InteractionRoleHint::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "generator" => InteractionRoleHint::Generator,
            "orchestrator" => InteractionRoleHint::Orchestrator,
            "test-harness" => InteractionRoleHint::TestHarness,
            "spec-document" => InteractionRoleHint::SpecDocument,
            "spawner" => InteractionRoleHint::Spawner,
            "documented-by" => InteractionRoleHint::DocumentedBy,
            "client" => InteractionRoleHint::Client,
            "server" => InteractionRoleHint::Server,
            "library" => InteractionRoleHint::Library,
            "tool" => InteractionRoleHint::Tool,
            _ => return None,
        })
    }

    pub fn all() -> &'static [InteractionRoleHint] {
        &[
            InteractionRoleHint::Generator,
            InteractionRoleHint::Orchestrator,
            InteractionRoleHint::TestHarness,
            InteractionRoleHint::SpecDocument,
            InteractionRoleHint::Spawner,
            InteractionRoleHint::DocumentedBy,
            InteractionRoleHint::Client,
            InteractionRoleHint::Server,
            InteractionRoleHint::Library,
            InteractionRoleHint::Tool,
        ]
    }
}

/// Interaction-surface record for one component. Matches Ravel-Lite's
/// `SurfaceRecord` shape with the sole field rename noted at the module
/// level.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SurfaceRecord {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub consumes_files: Vec<String>,
    #[serde(default)]
    pub produces_files: Vec<String>,
    #[serde(default)]
    pub network_endpoints: Vec<String>,
    #[serde(default)]
    pub data_formats: Vec<String>,
    #[serde(default)]
    pub external_tools_spawned: Vec<String>,
    #[serde(default)]
    pub explicit_cross_component_mentions: Vec<String>,
    #[serde(default)]
    pub interaction_role_hints: Vec<InteractionRoleHint>,
    #[serde(default)]
    pub notes: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_record_round_trips_via_yaml() {
        let original = SurfaceRecord {
            purpose: "Does the thing.".into(),
            consumes_files: vec!["~/.config/*.yaml".into()],
            produces_files: vec!["/tmp/out/*.json".into()],
            network_endpoints: vec!["grpc://svc:50051".into()],
            data_formats: vec!["FooRecord".into()],
            external_tools_spawned: vec!["git".into()],
            explicit_cross_component_mentions: vec!["Beta".into()],
            interaction_role_hints: vec![
                InteractionRoleHint::Generator,
                InteractionRoleHint::TestHarness,
            ],
            notes: String::new(),
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: SurfaceRecord = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn interaction_role_hint_every_variant_round_trips() {
        for hint in InteractionRoleHint::all() {
            let s = hint.as_str();
            assert_eq!(InteractionRoleHint::parse(s), Some(*hint));
            let yaml = serde_yaml::to_string(hint).unwrap();
            let parsed: InteractionRoleHint = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, *hint);
        }
    }

    #[test]
    fn unknown_interaction_role_hint_rejected_at_deserialisation() {
        let yaml = "- mystery-role\n";
        let err = serde_yaml::from_str::<Vec<InteractionRoleHint>>(yaml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("mystery-role") || msg.contains("unknown variant"),
            "unknown hint must be rejected: {msg}"
        );
    }

    #[test]
    fn surface_record_empty_body_defaults_all_fields() {
        let yaml = "purpose: p\n";
        let parsed: SurfaceRecord = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.purpose, "p");
        assert!(parsed.consumes_files.is_empty());
        assert!(parsed.interaction_role_hints.is_empty());
        assert!(parsed.explicit_cross_component_mentions.is_empty());
    }
}
