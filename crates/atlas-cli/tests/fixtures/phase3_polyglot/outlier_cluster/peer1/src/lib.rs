//! Peer member of the modularity-outlier subsystem fixture for Phase 3 PR-13.
//!
//! Each peer defines exactly one serde-derived `pub struct`, which the Rust
//! surface analyser converts into a single `defines-contract` edge. The
//! sibling `outlier` crate consumes all six peer contracts via the
//! workspace's `edges_add` overrides; that gives the outlier `Ce = 6` while
//! every peer keeps `Ce = 0`, which is enough to push the outlier above
//! the `>2σ` flag in a 7-member subsystem.

use serde::{Deserialize, Serialize};

/// Public serde struct → analyser emits one `defines-contract` edge with
/// id `peer1/peer-one` (the Rust analyser kebab-cases the struct name).
#[derive(Serialize, Deserialize)]
pub struct PeerOne {
    /// Trivial field — the public surface only needs *some* shape.
    pub value: u32,
}
