//! Parsed form of `defaults/component-kinds.yaml`, plus helpers that
//! render the kind and lifecycle catalogues into prompt-ready markdown.
//!
//! This file is to atlas-engine what
//! `component_ontology::defaults` is to the ontology crate: a small,
//! drift-guarded bridge between an authored YAML vocabulary and the
//! Rust enum surface the rest of the codebase pattern-matches on.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const COMPONENT_KINDS_SCHEMA_VERSION: u32 = 1;

/// The shipped `defaults/component-kinds.yaml`, embedded at compile
/// time. The path reaches the workspace-root `defaults/` directory; as
/// with the ontology crate, this is load-bearing only for path/git
/// deps, not crates.io publication.
pub const EMBEDDED_COMPONENT_KINDS_YAML: &str =
    include_str!("../../../defaults/component-kinds.yaml");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentKindsYaml {
    pub schema_version: u32,
    #[serde(default)]
    pub kinds: Vec<KindEntry>,
    #[serde(default)]
    pub lifecycle_scopes: Vec<LifecycleScopeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindEntry {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleScopeEntry {
    pub name: String,
    pub description: String,
}

pub fn parse(yaml: &str) -> Result<ComponentKindsYaml> {
    let parsed: ComponentKindsYaml =
        serde_yaml::from_str(yaml).context("failed to parse component-kinds YAML")?;
    if parsed.schema_version != COMPONENT_KINDS_SCHEMA_VERSION {
        anyhow::bail!(
            "component-kinds YAML schema_version is {}, expected {}",
            parsed.schema_version,
            COMPONENT_KINDS_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn parse_embedded() -> Result<ComponentKindsYaml> {
    parse(EMBEDDED_COMPONENT_KINDS_YAML)
}

/// Render the kind catalogue as the markdown bullet block that the
/// `{{COMPONENT_KINDS}}` token expands to. Each bullet is `- **`name`**
/// — flattened description`; a drift test elsewhere in this crate
/// checks the rendered names stay in bijection with [`ComponentKind`].
pub fn render_kinds_for_prompt(kinds_yaml: &ComponentKindsYaml) -> String {
    let mut out = String::new();
    for entry in &kinds_yaml.kinds {
        out.push_str(&format!(
            "- **`{}`** — {}\n",
            entry.name,
            flatten_paragraph(&entry.description),
        ));
    }
    out
}

pub fn render_lifecycle_scopes_for_prompt(kinds_yaml: &ComponentKindsYaml) -> String {
    let mut out = String::new();
    for entry in &kinds_yaml.lifecycle_scopes {
        out.push_str(&format!(
            "- **`{}`** — {}\n",
            entry.name,
            flatten_paragraph(&entry.description),
        ));
    }
    out
}

fn flatten_paragraph(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod drift_tests {
    //! Bijection between `defaults/component-kinds.yaml` and the
    //! [`ComponentKind`] Rust enum. Adding a kind in either place
    //! without the other fails the build.

    use super::*;
    use crate::types::ComponentKind;
    use component_ontology::LifecycleScope;
    use std::collections::BTreeSet;

    fn yaml_kind_names() -> BTreeSet<String> {
        let parsed = parse_embedded().expect("component-kinds YAML must parse");
        parsed.kinds.into_iter().map(|k| k.name).collect()
    }

    #[test]
    fn shipped_component_kinds_yaml_parses() {
        parse_embedded().unwrap();
    }

    #[test]
    fn component_kinds_in_yaml_and_rust_are_bijective() {
        let yaml = yaml_kind_names();
        let rust: BTreeSet<String> = ComponentKind::all()
            .iter()
            .map(|k| k.as_str().to_string())
            .collect();

        let missing_from_rust: Vec<_> = yaml.difference(&rust).cloned().collect();
        let missing_from_yaml: Vec<_> = rust.difference(&yaml).cloned().collect();

        assert!(
            missing_from_rust.is_empty(),
            "kind(s) in component-kinds.yaml but not in ComponentKind: {missing_from_rust:?}"
        );
        assert!(
            missing_from_yaml.is_empty(),
            "kind(s) in ComponentKind but not in component-kinds.yaml: {missing_from_yaml:?}"
        );
    }

    #[test]
    fn lifecycle_scopes_in_yaml_and_ontology_enum_are_bijective() {
        let parsed = parse_embedded().unwrap();
        let yaml: BTreeSet<String> = parsed
            .lifecycle_scopes
            .into_iter()
            .map(|l| l.name)
            .collect();
        let rust: BTreeSet<String> = LifecycleScope::all()
            .iter()
            .map(|l| l.as_str().to_string())
            .collect();
        assert_eq!(yaml, rust, "lifecycle-scope set divergence");
    }
}
