//! Peer member of the modularity-outlier subsystem fixture (see peer1).

use serde::{Deserialize, Serialize};

/// Public serde struct → contract id `peer4/peer-four`.
#[derive(Serialize, Deserialize)]
pub struct PeerFour {
    pub value: u32,
}
