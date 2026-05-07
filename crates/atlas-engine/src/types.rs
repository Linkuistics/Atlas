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

use std::collections::BTreeSet;
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
    RustProcMacro,
    Workspace,
    NodeLibrary,
    NodeCli,
    TypescriptPackage,
    JavascriptPackage,
    ReactApp,
    ReactLibrary,
    PythonLibrary,
    PythonApp,
    PythonPackage,
    ElixirProject,
    RacketPackage,
    LispkitPackage,
    DartLibrary,
    DartApp,
    /// A Dart package — `pubspec.yaml` present with no `flutter:` top-level
    /// block. Recognised deterministically at L3 by the `dart-classifier`.
    /// Phase 2 PR-7.
    DartPackage,
    FlutterApp,
    /// A Flutter package — `pubspec.yaml` with a `flutter:` top-level block.
    /// Recognised deterministically at L3 by the `dart-classifier`.
    /// Phase 2 PR-7.
    FlutterPackage,
    CsharpProject,
    CsharpSolution,
    DotnetLibrary,
    DotnetService,
    Website,
    Service,
    DockerImage,
    DockerComposeBundle,
    ComposeOrchestration,
    Installer,
    ShellScript,
    MakefileOrchestration,
    ShellScripts,
    SqlScripts,
    CodegenTool,
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
            ComponentKind::RustProcMacro => "rust-proc-macro",
            ComponentKind::Workspace => "workspace",
            ComponentKind::NodeLibrary => "node-library",
            ComponentKind::NodeCli => "node-cli",
            ComponentKind::TypescriptPackage => "typescript-package",
            ComponentKind::JavascriptPackage => "javascript-package",
            ComponentKind::ReactApp => "react-app",
            ComponentKind::ReactLibrary => "react-library",
            ComponentKind::PythonLibrary => "python-library",
            ComponentKind::PythonApp => "python-app",
            ComponentKind::PythonPackage => "python-package",
            ComponentKind::ElixirProject => "elixir-project",
            ComponentKind::RacketPackage => "racket-package",
            ComponentKind::LispkitPackage => "lispkit-package",
            ComponentKind::DartLibrary => "dart-library",
            ComponentKind::DartApp => "dart-app",
            ComponentKind::DartPackage => "dart-package",
            ComponentKind::FlutterApp => "flutter-app",
            ComponentKind::FlutterPackage => "flutter-package",
            ComponentKind::CsharpProject => "csharp-project",
            ComponentKind::CsharpSolution => "csharp-solution",
            ComponentKind::DotnetLibrary => "dotnet-library",
            ComponentKind::DotnetService => "dotnet-service",
            ComponentKind::Website => "website",
            ComponentKind::Service => "service",
            ComponentKind::DockerImage => "docker-image",
            ComponentKind::DockerComposeBundle => "docker-compose-bundle",
            ComponentKind::ComposeOrchestration => "compose-orchestration",
            ComponentKind::Installer => "installer",
            ComponentKind::ShellScript => "shell-script",
            ComponentKind::MakefileOrchestration => "makefile-orchestration",
            ComponentKind::ShellScripts => "shell-scripts",
            ComponentKind::SqlScripts => "sql-scripts",
            ComponentKind::CodegenTool => "codegen-tool",
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
            "rust-proc-macro" => ComponentKind::RustProcMacro,
            "workspace" => ComponentKind::Workspace,
            "node-library" => ComponentKind::NodeLibrary,
            "node-cli" => ComponentKind::NodeCli,
            "typescript-package" => ComponentKind::TypescriptPackage,
            "javascript-package" => ComponentKind::JavascriptPackage,
            "react-app" => ComponentKind::ReactApp,
            "react-library" => ComponentKind::ReactLibrary,
            "python-library" => ComponentKind::PythonLibrary,
            "python-app" => ComponentKind::PythonApp,
            "python-package" => ComponentKind::PythonPackage,
            "elixir-project" => ComponentKind::ElixirProject,
            "racket-package" => ComponentKind::RacketPackage,
            "lispkit-package" => ComponentKind::LispkitPackage,
            "dart-library" => ComponentKind::DartLibrary,
            "dart-app" => ComponentKind::DartApp,
            "dart-package" => ComponentKind::DartPackage,
            "flutter-app" => ComponentKind::FlutterApp,
            "flutter-package" => ComponentKind::FlutterPackage,
            "csharp-project" => ComponentKind::CsharpProject,
            "csharp-solution" => ComponentKind::CsharpSolution,
            "dotnet-library" => ComponentKind::DotnetLibrary,
            "dotnet-service" => ComponentKind::DotnetService,
            "website" => ComponentKind::Website,
            "service" => ComponentKind::Service,
            "docker-image" => ComponentKind::DockerImage,
            "docker-compose-bundle" => ComponentKind::DockerComposeBundle,
            "compose-orchestration" => ComponentKind::ComposeOrchestration,
            "installer" => ComponentKind::Installer,
            "shell-script" => ComponentKind::ShellScript,
            "makefile-orchestration" => ComponentKind::MakefileOrchestration,
            "shell-scripts" => ComponentKind::ShellScripts,
            "sql-scripts" => ComponentKind::SqlScripts,
            "codegen-tool" => ComponentKind::CodegenTool,
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
            ComponentKind::RustProcMacro,
            ComponentKind::Workspace,
            ComponentKind::NodeLibrary,
            ComponentKind::NodeCli,
            ComponentKind::TypescriptPackage,
            ComponentKind::JavascriptPackage,
            ComponentKind::ReactApp,
            ComponentKind::ReactLibrary,
            ComponentKind::PythonLibrary,
            ComponentKind::PythonApp,
            ComponentKind::PythonPackage,
            ComponentKind::ElixirProject,
            ComponentKind::RacketPackage,
            ComponentKind::LispkitPackage,
            ComponentKind::DartLibrary,
            ComponentKind::DartApp,
            ComponentKind::DartPackage,
            ComponentKind::FlutterApp,
            ComponentKind::FlutterPackage,
            ComponentKind::CsharpProject,
            ComponentKind::CsharpSolution,
            ComponentKind::DotnetLibrary,
            ComponentKind::DotnetService,
            ComponentKind::Website,
            ComponentKind::Service,
            ComponentKind::DockerImage,
            ComponentKind::DockerComposeBundle,
            ComponentKind::ComposeOrchestration,
            ComponentKind::Installer,
            ComponentKind::ShellScript,
            ComponentKind::MakefileOrchestration,
            ComponentKind::ShellScripts,
            ComponentKind::SqlScripts,
            ComponentKind::CodegenTool,
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
///
/// Atlas vNext: `languages` is a set, not a single value. A polyglot
/// component (e.g., a Rust crate with embedded Python tooling) carries
/// every language in one record (design §3.1). For Phase 1 most
/// components have a single-element set; the multi-language path is
/// dormant until subsequent analyser PRs populate it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Classification {
    pub kind: ComponentKind,
    pub languages: BTreeSet<String>,
    pub build_system: Option<String>,
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub role: Option<String>,
    pub evidence_grade: EvidenceGrade,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
    pub is_boundary: bool,
    /// Stable id of the L3 analyser whose verdict produced this
    /// classification (e.g. `cargo-toml-classifier`, `dockerfile-l3`).
    /// `"none"` when no analyser took the candidate (the fall-through
    /// classification path) and `"override"` for hand-authored pins
    /// or `overrides.additions` entries that bypass the registry.
    pub analyser_id: String,
    /// Free-form analyser version reported by the dispatching
    /// analyser's [`atlas_analyzers::Analyzer::version`]. Pairs with
    /// `analyser_id`; `"0.0.0"` on the all-declined / override paths.
    pub analyser_version: String,
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
