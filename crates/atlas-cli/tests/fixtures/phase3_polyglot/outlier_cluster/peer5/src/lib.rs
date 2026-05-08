//! Peer member of the modularity-outlier subsystem fixture (see peer1).

use serde::{Deserialize, Serialize};

/// Public serde struct → contract id `peer5/peer-five`.
#[derive(Serialize, Deserialize)]
pub struct PeerFive {
    pub value: u32,
}
