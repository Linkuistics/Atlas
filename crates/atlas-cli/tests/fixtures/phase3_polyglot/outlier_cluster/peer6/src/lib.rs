//! Peer member of the modularity-outlier subsystem fixture (see peer1).

use serde::{Deserialize, Serialize};

/// Public serde struct → contract id `peer6/peer-six`.
#[derive(Serialize, Deserialize)]
pub struct PeerSix {
    pub value: u32,
}
