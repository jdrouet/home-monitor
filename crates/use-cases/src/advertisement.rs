use std::collections::HashMap;

use domain::{MacAddress, Timestamp};

/// One Bluetooth LE advertisement, stamped by the adapter on receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    pub address: MacAddress,
    pub received_at: Timestamp,
    /// Service data keyed by 16-bit service UUID.
    pub service_data: HashMap<u16, Vec<u8>>,
}
