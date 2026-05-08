//! Peer member of the modularity-outlier subsystem fixture (see peer1).

use serde::{Deserialize, Serialize};

/// Public serde struct → contract id `peer3/peer-three`.
#[derive(Serialize, Deserialize)]
pub struct PeerThree {
    pub value: u32,
}
