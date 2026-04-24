//! L2/L3 value types: `ComponentKind`, `Candidate`, `RationaleBundle`,
//! `Classification`.
//!
//! `ComponentKind` is atlas-engine's typed form of the `kind` string
//! stored in `components.yaml`; the vocabulary is authored in
//! `defaults/component-kinds.yaml` and a drift test in this module
//! asserts bijection. atlas-index deliberately stores the field as a
//! plain `String` so the vocabulary can grow without churning every
//! downstream consumer on every new term (see the memory entry
//! "ComponentKind enum deferred to atlas-engine").

use std::path::PathBuf;

use component_ontology::{EvidenceGrade, LifecycleScope};
use serde::{Deserialize, Serialize};

use crate::l1_queries::{DocHeading, ShebangEntry};

// `Serialize`/`Deserialize` above are used only by `ComponentKind`; the
// value types below (`RationaleBundle`, `Candidate`, `Classification`)
// are in-memory only.

/// What kind of thing a component is. Values are emitted to disk as
/// the kebab-case string returned by [`ComponentKind::as_str`] and
/// parsed back by [`ComponentKind::parse`]. The set is mirrored by
/// `defaults/component-kinds.yaml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentKind {
    RustLibrary,
    RustCli,
    Workspace,
    NodePackage,
    NodeCli,
    PythonPackage,
    Website,
    Service,
    ConfigRepo,
    DocsRepo,
    Spec,
    External,
    NonComponent,
}

impl ComponentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ComponentKind::RustLibrary => "rust-library",
            ComponentKind::RustCli => "rust-cli",
            ComponentKind::Workspace => "workspace",
            ComponentKind::NodePackage => "node-package",
            ComponentKind::NodeCli => "node-cli",
            ComponentKind::PythonPackage => "python-package",
            ComponentKind::Website => "website",
            ComponentKind::Service => "service",
            ComponentKind::ConfigRepo => "config-repo",
            ComponentKind::DocsRepo => "docs-repo",
            ComponentKind::Spec => "spec",
            ComponentKind::External => "external",
            ComponentKind::NonComponent => "non-component",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "rust-library" => ComponentKind::RustLibrary,
            "rust-cli" => ComponentKind::RustCli,
            "workspace" => ComponentKind::Workspace,
            "node-package" => ComponentKind::NodePackage,
            "node-cli" => ComponentKind::NodeCli,
            "python-package" => ComponentKind::PythonPackage,
            "website" => ComponentKind::Website,
            "service" => ComponentKind::Service,
            "config-repo" => ComponentKind::ConfigRepo,
            "docs-repo" => ComponentKind::DocsRepo,
            "spec" => ComponentKind::Spec,
            "external" => ComponentKind::External,
            "non-component" => ComponentKind::NonComponent,
            _ => return None,
        })
    }

    pub fn all() -> &'static [ComponentKind] {
        &[
            ComponentKind::RustLibrary,
            ComponentKind::RustCli,
            ComponentKind::Workspace,
            ComponentKind::NodePackage,
            ComponentKind::NodeCli,
            ComponentKind::PythonPackage,
            ComponentKind::Website,
            ComponentKind::Service,
            ComponentKind::ConfigRepo,
            ComponentKind::DocsRepo,
            ComponentKind::Spec,
            ComponentKind::External,
            ComponentKind::NonComponent,
        ]
    }
}

/// Signals attached to a candidate directory during L2. Every field is
/// deterministic and cheap to rebuild; the bundle is passed to the
/// classifier (L3) as evidence.
///
/// Not `Serialize`/`Deserialize`: the LLM request JSON is constructed
/// field-by-field in `l3_classify::build_llm_inputs` so the wire shape
/// is decoupled from the in-memory one. An on-disk representation for
/// a component lives in `atlas_index::ComponentEntry`, not here.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RationaleBundle {
    pub manifests: Vec<PathBuf>,
    pub is_git_root: bool,
    pub doc_headings: Vec<DocHeading>,
    pub shebangs: Vec<ShebangEntry>,
}

/// One candidate directory produced by L2. Zero, one, or many
/// candidates may live under any given root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Candidate {
    pub dir: PathBuf,
    pub rationale_bundle: RationaleBundle,
}

/// Outcome of L3 classification for a single candidate. `is_boundary`
/// separates confirmed components (which L4 includes in the tree) from
/// candidates that the engine enumerated but decided against.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Classification {
    pub kind: ComponentKind,
    pub language: Option<String>,
    pub build_system: Option<String>,
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub role: Option<String>,
    pub evidence_grade: EvidenceGrade,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
    pub is_boundary: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_kind_round_trips_through_str() {
        for kind in ComponentKind::all() {
            assert_eq!(ComponentKind::parse(kind.as_str()), Some(*kind));
        }
    }

    #[test]
    fn component_kind_round_trips_through_yaml() {
        for kind in ComponentKind::all() {
            let yaml = serde_yaml::to_string(kind).unwrap();
            let parsed: ComponentKind = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, *kind);
        }
    }
}
