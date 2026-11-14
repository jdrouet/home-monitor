use serde::Deserialize;

use crate::{MacAddress, RoomName};

/// A configured sensor and the room it sits in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thermometer {
    pub mac: MacAddress,
    pub room: RoomName,
}
