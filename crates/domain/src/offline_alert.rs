use serde::{Deserialize, Serialize};

use crate::Timestamp;

/// The offline alert last delivered for a sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineAlert {
    pub last_seen: Timestamp,
    pub notified_at: Timestamp,
}
