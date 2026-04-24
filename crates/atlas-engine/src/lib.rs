//! Salsa-backed query graph that drives Atlas's component-discovery
//! fixedpoint (L0 inputs through L9 projections).
//!
//! Layer status (tracked by backlog tasks):
//!
//! - L0 inputs and L1 enumeration queries — live.
//! - L2 candidate generation, L3 classification — live.
//! - L8 subcarve back-edge — stubbed; real implementation lands in
//!   the L7/L8 backlog task.
//! - L4–L7, L9, CLI — upcoming tasks.

pub mod db;
pub mod defaults;
pub mod heuristics;
pub mod ingest;
pub mod l1_queries;
pub mod l2_candidates;
pub mod l3_classify;
pub mod l8_recurse;
pub mod manifest_parse;
pub mod manifest_patterns;
pub mod types;

pub use db::{AtlasDatabase, ExecutedEvent, File, Workspace};
pub use defaults::{
    parse as parse_component_kinds_yaml, parse_embedded as parse_embedded_component_kinds_yaml,
    render_kinds_for_prompt, render_lifecycle_scopes_for_prompt, ComponentKindsYaml,
    EMBEDDED_COMPONENT_KINDS_YAML,
};
pub use ingest::{seed_filesystem, seed_filesystem_with_limit, DEFAULT_BINARY_SIZE_LIMIT};
pub use l1_queries::{
    doc_headings, file_content, file_tree_sha, git_boundaries, manifests_in, shebangs, DocHeading,
    ShebangEntry,
};
pub use l2_candidates::candidate_components_at;
pub use l3_classify::is_component;
pub use l8_recurse::subcarve_plan;
pub use manifest_patterns::is_manifest_file;
pub use types::{Candidate, Classification, ComponentKind, RationaleBundle};
