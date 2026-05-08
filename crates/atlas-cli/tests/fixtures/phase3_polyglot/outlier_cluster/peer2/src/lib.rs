//! Peer member of the modularity-outlier subsystem fixture (see peer1).

use serde::{Deserialize, Serialize};

/// Public serde struct → contract id `peer2/peer-two`.
#[derive(Serialize, Deserialize)]
pub struct PeerTwo {
    pub value: u32,
}
