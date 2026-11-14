use serde::{Deserialize, Serialize};

use crate::{Deviation, Timestamp};

/// The temperature alert last delivered for a room.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TemperatureAlert {
    pub deviation: Deviation,
    /// The last temperature known when the notification was delivered.
    pub celsius: f64,
    pub notified_at: Timestamp,
}
