use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{OfflineAlert, ResolvedAlert, SourceId, TemperatureAlert};

/// The alerts delivered and not yet resolved, per sensor, and the resolved
/// ones still kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertState {
    pub offline: BTreeMap<SourceId, OfflineAlert>,
    pub temperature: BTreeMap<SourceId, TemperatureAlert>,
    /// Resolved alerts, oldest first.
    #[serde(default)]
    pub history: Vec<ResolvedAlert>,
}

#[cfg(test)]
mod tests {
    use super::AlertState;

    #[test]
    fn a_state_without_history_has_none() {
        let state: AlertState =
            serde_json::from_str(r#"{"offline":{},"temperature":{}}"#).expect("parses");
        assert!(state.history.is_empty());
    }
}
