//! Salsa-backed query graph that drives Atlas's component-discovery
//! fixedpoint (L0 inputs through L9 projections).
//!
//! Layer status (tracked by backlog tasks):
//!
//! - L0 inputs and L1 enumeration queries — live.
//! - L2 candidate generation, L3 classification — live.
//! - L4 tree assembly + rename-match — live.
//! - L5 surface extraction, L6 candidate-edge proposal — live.
//! - L7 graph-structural analysis, L8 sub-carve decision + fixedpoint
//!   driver — live.
//! - L9 projections — live.
//! - CLI — lives in the `atlas-cli` crate.

pub mod db;
pub mod defaults;
pub mod fixedpoint;
pub mod heuristics;
pub mod identifiers;
pub mod ingest;
pub mod l1_queries;
pub mod l2_candidates;
pub mod l3_classify;
pub mod l4_tree;
pub mod l5_surface;
pub mod l6_edges;
pub mod l7_structural;
pub mod l8_recurse;
pub mod l9_projections;
pub mod l9_subsystems;
pub mod llm_cache;
pub mod manifest_parse;
pub mod manifest_patterns;
pub mod progress;
pub mod subcarve_policy;
pub mod surface_types;
pub mod types;

pub use db::{AtlasDatabase, ExecutedEvent, File, Workspace, DEFAULT_MAX_DEPTH};
pub use defaults::{
    parse as parse_component_kinds_yaml, parse_embedded as parse_embedded_component_kinds_yaml,
    render_kinds_for_prompt, render_lifecycle_scopes_for_prompt, ComponentKindsYaml,
    EMBEDDED_COMPONENT_KINDS_YAML,
};
pub use fixedpoint::{run_fixedpoint, FixedpointConfig, FixedpointResult, FIXEDPOINT_HARD_CAP};
pub use ingest::{
    seed_filesystem, seed_filesystem_excluding, seed_filesystem_with_limit,
    DEFAULT_BINARY_SIZE_LIMIT,
};
pub use l1_queries::{
    doc_headings, file_content, file_tree_sha, git_boundaries, manifests_in, shebangs, DocHeading,
    ShebangEntry,
};
pub use l2_candidates::candidate_components_at;
pub use l3_classify::is_component;
pub use l4_tree::{
    all_components, component_children, component_parent, component_path_segments, try_assemble,
    TreeAssemblyError,
};
pub use l5_surface::{surface_of, EMBEDDED_STAGE1_SURFACE_PROMPT};
pub use l6_edges::{all_proposed_edges, candidate_edges_for, EMBEDDED_STAGE2_EDGES_PROMPT};
pub use l7_structural::{
    cliques, edge_graph, modularity_hint, sccs, seam_density, Clique, EdgeGraph, ModularityHint,
    Scc,
};
pub use l8_recurse::{
    should_subcarve, subcarve_decision, subcarve_plan, SubcarveDecision, EMBEDDED_SUBCARVE_PROMPT,
};
pub use l9_projections::{
    components_yaml_snapshot, components_yaml_snapshot_with_prompt_shas,
    external_components_yaml_snapshot, externals_from_manifests, known_component_ids,
    related_components_yaml_snapshot, sha256_hex, PROMPT_ID_STRINGS,
};
pub use l9_subsystems::{
    check_subsystem_id_members, check_subsystem_namespace, subsystems_yaml_snapshot,
};
pub use llm_cache::{LlmCacheKey, LlmResponseCache};
pub use manifest_patterns::is_manifest_file;
pub use progress::{relpath_of, Phase, ProgressEvent, ProgressSink, PromptBreakdown};
pub use surface_types::{InteractionRoleHint, SurfaceRecord};
pub use types::{Candidate, Classification, ComponentKind, RationaleBundle};
