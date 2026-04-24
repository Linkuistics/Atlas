//! Salsa-backed query graph that drives Atlas's component-discovery
//! fixedpoint (L0 inputs through L9 projections).
//!
//! This crate currently wires up L0 (file and workspace inputs) and L1
//! (deterministic enumeration queries); higher layers land in later
//! backlog tasks.

pub mod db;
pub mod ingest;
pub mod l1_queries;
pub mod manifest_patterns;

pub use db::{AtlasDatabase, ExecutedEvent, File, Workspace};
pub use ingest::{seed_filesystem, seed_filesystem_with_limit, DEFAULT_BINARY_SIZE_LIMIT};
pub use l1_queries::{
    doc_headings, file_content, file_tree_sha, git_boundaries, manifests_in, shebangs, DocHeading,
    ShebangEntry,
};
pub use manifest_patterns::is_manifest_file;
