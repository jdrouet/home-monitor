use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{LoginState, OfflineAlert, ResolvedAlert, SourceId, TemperatureAlert};

/// The alerts delivered and not yet resolved, per sensor, and the resolved
/// ones still kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertState {
    pub offline: BTreeMap<SourceId, OfflineAlert>,
    pub temperature: BTreeMap<SourceId, TemperatureAlert>,
    /// Resolved alerts, oldest first.
    #[serde(default)]
    pub history: Vec<ResolvedAlert>,
    /// The Garmin login watch, absent until a session reading arrives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub garmin_login: Option<LoginState>,
}

#[cfg(test)]
mod tests {
    use super::AlertState;

    #[test]
    fn a_state_without_history_has_none() {
        let state: AlertState =
            serde_json::from_str(r#"{"offline":{},"temperature":{}}"#).expect("parses");
        assert!(state.history.is_empty());
        assert_eq!(state.garmin_login, None);
    }

    #[test]
    fn an_absent_garmin_login_is_not_written() {
        let json = serde_json::to_string(&AlertState::default()).expect("serializes");
        assert!(!json.contains("garmin_login"));
    }
}
