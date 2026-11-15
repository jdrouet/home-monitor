use serde::{Deserialize, Serialize};

use crate::MacAddress;

/// The source id of the Garmin account's health data.
pub const GARMIN: &str = "garmin";

/// Identifies the device a measurement came from, such as a MAC address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SourceId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<MacAddress> for SourceId {
    fn from(address: MacAddress) -> Self {
        Self(address.to_string())
    }
}
