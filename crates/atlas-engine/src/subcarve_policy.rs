//! Per-kind tabulated policy for L8's sub-carve decision.
//!
//! Matches the style of [`crate::heuristics`]: one arm per
//! [`ComponentKind`], each returning a [`PolicyDecision`] derived from
//! the signals the engine already holds. Kinds that never recurse
//! return [`PolicyDecision::Stop`]; kinds whose structure might recurse
//! return [`PolicyDecision::Recurse`] when the signals are clear or
//! [`PolicyDecision::AskLlm`] when they are not.
//!
//! This separation keeps the LLM escalation tiny: the deterministic
//! table handles most components and the LLM is only consulted when
//! the policy genuinely cannot decide.

use crate::l7_structural::{Clique, ModularityHint};
use crate::types::ComponentKind;

/// Hard depth cap for library-like kinds whose signals do not fire.
/// Bumped to `max_depth` when [`ModularityHint`] is present — the
/// modularity signal is strong evidence that deeper structure exists.
const LIBRARY_DEFAULT_DEPTH_CAP: u32 = 3;

/// Inputs fed to [`decide`]. Carrying the signals in a struct keeps the
/// call site — [`crate::l8_recurse::should_subcarve`] — readable.
#[derive(Debug, Clone)]
pub struct SubcarveSignals {
    pub kind: ComponentKind,
    pub current_depth: u32,
    pub max_depth: u32,
    pub seam_density: f32,
    pub modularity_hint: Option<ModularityHint>,
    pub cliques_touching: Vec<Clique>,
    pub pin_suppressed_children: Vec<String>,
}

/// Three-way verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Do not recurse under this component.
    Stop,
    /// Recurse — policy is confident without LLM help. `subcarve_plan`
    /// may still ask the LLM to identify *which* sub-dirs, but whether
    /// to recurse is settled.
    Recurse,
    /// Policy cannot decide; defer to the LLM via `PromptId::Subcarve`.
    AskLlm,
}

/// Apply the per-kind policy table.
pub fn decide(signals: &SubcarveSignals) -> PolicyDecision {
    // Universal cap: --max-depth wins over every per-kind rule. Also
    // respects the convention that depth is 0-indexed at the root.
    if signals.current_depth >= signals.max_depth {
        return PolicyDecision::Stop;
    }

    match signals.kind {
        // Leaf-like kinds: one component end-to-end, never carve inside.
        ComponentKind::RustCli
        | ComponentKind::NodeCli
        | ComponentKind::Service
        | ComponentKind::Website => PolicyDecision::Stop,

        // Documentation and configuration repositories are treated as
        // single components — their internal structure is a directory
        // hierarchy, not a call graph.
        ComponentKind::Spec | ComponentKind::DocsRepo | ComponentKind::ConfigRepo => {
            PolicyDecision::Stop
        }

        // Workspaces trivially contain children (their members). Always
        // recurse; the member directories feed L2 as candidate roots.
        ComponentKind::Workspace => PolicyDecision::Recurse,

        // Libraries: recurse up to LIBRARY_DEFAULT_DEPTH_CAP by
        // default; extend to --max-depth when modularity_hint fires.
        // Below the cap, route to the LLM for sub-dir identification.
        ComponentKind::RustLibrary | ComponentKind::NodePackage | ComponentKind::PythonPackage => {
            let cap = if signals.modularity_hint.is_some() {
                signals.max_depth
            } else {
                LIBRARY_DEFAULT_DEPTH_CAP.min(signals.max_depth)
            };
            if signals.current_depth >= cap {
                PolicyDecision::Stop
            } else if signals.modularity_hint.is_some() {
                // Hint is clear — recurse deterministically.
                PolicyDecision::Recurse
            } else {
                PolicyDecision::AskLlm
            }
        }

        // External components live in external-components.yaml; Atlas
        // does not own their internals. NonComponents have already
        // been marked `is_boundary: false` and should never reach here.
        ComponentKind::External | ComponentKind::NonComponent => PolicyDecision::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals_for(kind: ComponentKind) -> SubcarveSignals {
        SubcarveSignals {
            kind,
            current_depth: 0,
            max_depth: 8,
            seam_density: 0.0,
            modularity_hint: None,
            cliques_touching: Vec::new(),
            pin_suppressed_children: Vec::new(),
        }
    }

    #[test]
    fn rust_cli_always_stops_regardless_of_signals() {
        let mut signals = signals_for(ComponentKind::RustCli);
        signals.modularity_hint = Some(ModularityHint {
            partition_a: vec!["a".into()],
            partition_b: vec!["b".into()],
            cross_edges: 0,
            total_internal_edges: 4,
        });
        assert_eq!(decide(&signals), PolicyDecision::Stop);
    }

    #[test]
    fn node_cli_stops() {
        assert_eq!(
            decide(&signals_for(ComponentKind::NodeCli)),
            PolicyDecision::Stop
        );
    }

    #[test]
    fn service_stops() {
        assert_eq!(
            decide(&signals_for(ComponentKind::Service)),
            PolicyDecision::Stop
        );
    }

    #[test]
    fn spec_docs_config_stop() {
        for kind in [
            ComponentKind::Spec,
            ComponentKind::DocsRepo,
            ComponentKind::ConfigRepo,
        ] {
            assert_eq!(decide(&signals_for(kind)), PolicyDecision::Stop);
        }
    }

    #[test]
    fn website_stops() {
        assert_eq!(
            decide(&signals_for(ComponentKind::Website)),
            PolicyDecision::Stop
        );
    }

    #[test]
    fn workspace_always_recurses_under_the_depth_cap() {
        let signals = signals_for(ComponentKind::Workspace);
        assert_eq!(decide(&signals), PolicyDecision::Recurse);
    }

    #[test]
    fn rust_library_with_no_hint_asks_llm() {
        let signals = signals_for(ComponentKind::RustLibrary);
        assert_eq!(decide(&signals), PolicyDecision::AskLlm);
    }

    #[test]
    fn rust_library_with_modularity_hint_recurses_deterministically() {
        let mut signals = signals_for(ComponentKind::RustLibrary);
        signals.modularity_hint = Some(ModularityHint {
            partition_a: vec!["a".into(), "b".into()],
            partition_b: vec!["c".into(), "d".into()],
            cross_edges: 0,
            total_internal_edges: 4,
        });
        assert_eq!(decide(&signals), PolicyDecision::Recurse);
    }

    #[test]
    fn max_depth_zero_forces_stop_on_every_kind() {
        for kind in [
            ComponentKind::RustLibrary,
            ComponentKind::Workspace,
            ComponentKind::NodePackage,
            ComponentKind::PythonPackage,
        ] {
            let mut signals = signals_for(kind);
            signals.max_depth = 0;
            signals.current_depth = 0;
            assert_eq!(
                decide(&signals),
                PolicyDecision::Stop,
                "--max-depth=0 must stop kind {kind:?}"
            );
        }
    }

    #[test]
    fn library_depth_cap_is_three_without_modularity_hint() {
        let mut signals = signals_for(ComponentKind::RustLibrary);
        signals.max_depth = 8;
        signals.current_depth = 2;
        // Below the default cap of 3 → AskLlm.
        assert_eq!(decide(&signals), PolicyDecision::AskLlm);
        signals.current_depth = 3;
        assert_eq!(decide(&signals), PolicyDecision::Stop);
    }

    #[test]
    fn library_depth_cap_extends_to_max_depth_with_modularity_hint() {
        let mut signals = signals_for(ComponentKind::RustLibrary);
        signals.max_depth = 8;
        signals.current_depth = 5; // deeper than the 3-default cap
        signals.modularity_hint = Some(ModularityHint {
            partition_a: vec!["a".into(), "b".into()],
            partition_b: vec!["c".into(), "d".into()],
            cross_edges: 0,
            total_internal_edges: 4,
        });
        assert_eq!(decide(&signals), PolicyDecision::Recurse);
    }

    #[test]
    fn non_component_stops() {
        assert_eq!(
            decide(&signals_for(ComponentKind::NonComponent)),
            PolicyDecision::Stop
        );
    }
}
