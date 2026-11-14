use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{OfflineAlert, SourceId, TemperatureAlert};

/// The alerts delivered and not yet resolved, per sensor.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertState {
    pub offline: BTreeMap<SourceId, OfflineAlert>,
    pub temperature: BTreeMap<SourceId, TemperatureAlert>,
}
