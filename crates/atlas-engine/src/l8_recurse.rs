//! Stub for L8 recursion (sub-carving). The full implementation lands
//! in the task-8 backlog entry ("Implement atlas-engine L7 and L8");
//! this file exists so that L2 can read [`subcarve_plan`] via a Salsa
//! tracked query today and the real implementation replaces this stub
//! without changing call sites.
//!
//! Until then, every component sub-carves to the empty set — i.e. no
//! recursion, the same behaviour the task spec asks for as the stub.

use std::path::PathBuf;
use std::sync::Arc;

use crate::db::Workspace;

/// Directories to treat as new L2 candidate roots inside the component
/// `parent_component_id`. Returns an empty list in the stub
/// implementation — the real L8 policy lands in task 8.
#[salsa::tracked]
pub fn subcarve_plan<'db>(
    _db: &'db dyn salsa::Database,
    _workspace: Workspace,
    _parent_component_id: String,
) -> Arc<Vec<PathBuf>> {
    Arc::new(Vec::new())
}
