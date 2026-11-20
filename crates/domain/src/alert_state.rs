use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    ImportJob, LoginState, OfflineAlert, ResolvedAlert, SourceId, TemperatureAlert, Timestamp,
};

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
    /// The newest give-up notified per Garmin job, as its `retry_at`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub garmin_imports: BTreeMap<ImportJob, Timestamp>,
}

#[cfg(test)]
mod tests {
    use crate::{ImportJob, Timestamp};

    use super::AlertState;

    #[test]
    fn a_state_without_history_has_none() {
        let state: AlertState =
            serde_json::from_str(r#"{"offline":{},"temperature":{}}"#).expect("parses");
        assert!(state.history.is_empty());
        assert_eq!(state.garmin_login, None);
        assert!(state.garmin_imports.is_empty());
    }

    #[test]
    fn an_empty_import_map_is_not_written_and_a_filled_one_round_trips() {
        let json = serde_json::to_string(&AlertState::default()).expect("serializes");
        assert!(!json.contains("garmin_imports"));
        let mut state = AlertState::default();
        state
            .garmin_imports
            .insert(ImportJob::Workouts, Timestamp::from_micros(7));
        let json = serde_json::to_string(&state).expect("serializes");
        assert!(json.contains(r#""garmin_imports":{"workouts":7}"#));
        let back: AlertState = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, state);
    }

    #[test]
    fn an_absent_garmin_login_is_not_written() {
        let json = serde_json::to_string(&AlertState::default()).expect("serializes");
        assert!(!json.contains("garmin_login"));
    }
}
