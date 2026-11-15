use serde::{Deserialize, Serialize};

use crate::Timestamp;

/// What the alerter keeps of the Garmin login watch: the newest failed and
/// newest working session, and the episode last notified, if any.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok_at: Option<Timestamp>,
    /// When the first notification of this episode was delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified_at: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use crate::Timestamp;

    use super::LoginState;

    #[test]
    fn empty_fields_are_omitted_and_restored() {
        let state = LoginState {
            failed_at: Some(Timestamp::from_micros(10)),
            ..LoginState::default()
        };
        let json = serde_json::to_string(&state).expect("serializes");
        assert_eq!(json, r#"{"failed_at":10}"#);
        assert_eq!(
            serde_json::from_str::<LoginState>(&json).expect("parses"),
            state
        );
    }
}
