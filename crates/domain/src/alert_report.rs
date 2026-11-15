use serde::{Deserialize, Serialize};

use crate::{AlertReportError, AlertState};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct WireOut<'a> {
    version: u32,
    #[serde(flatten)]
    state: &'a AlertState,
}

#[derive(Deserialize)]
struct WireIn {
    version: u32,
    #[serde(flatten)]
    state: AlertState,
}

pub fn encode(state: &AlertState) -> Vec<u8> {
    serde_json::to_vec(&WireOut {
        version: SCHEMA_VERSION,
        state,
    })
    .expect("an alert report always serializes")
}

pub fn decode(payload: &[u8]) -> Result<AlertState, AlertReportError> {
    let wire: WireIn = serde_json::from_slice(payload)?;
    if wire.version != SCHEMA_VERSION {
        return Err(AlertReportError::UnsupportedVersion(wire.version));
    }
    Ok(wire.state)
}

#[cfg(test)]
mod tests {
    use crate::{
        AlertKind, AlertReportError, AlertState, OfflineAlert, ResolvedAlert, SourceId, Timestamp,
    };

    use super::{decode, encode};

    fn sample() -> AlertState {
        let mut state = AlertState::default();
        state.offline.insert(
            SourceId::from("A4:C1:38:00:00:01".to_owned()),
            OfflineAlert {
                last_seen: Timestamp::from_micros(10),
                since: Timestamp::from_micros(20),
                notified_at: Timestamp::from_micros(30),
            },
        );
        state.history.push(ResolvedAlert {
            source_id: SourceId::from("A4:C1:38:00:00:02".to_owned()),
            kind: AlertKind::TooHot,
            started: Timestamp::from_micros(40),
            ended: Timestamp::from_micros(50),
        });
        state
    }

    #[test]
    fn encoding_matches_the_wire_format() {
        let expected = r#"{"version":1,"offline":{"A4:C1:38:00:00:01":{"last_seen":10,"since":20,"notified_at":30}},"temperature":{},"history":[{"source_id":"A4:C1:38:00:00:02","kind":"too_hot","started":40,"ended":50}]}"#;
        assert_eq!(encode(&sample()), expected.as_bytes());
    }

    #[test]
    fn round_trips() {
        assert_eq!(decode(&encode(&sample())).expect("decodes"), sample());
    }

    #[test]
    fn other_versions_are_rejected() {
        let result = decode(br#"{"version":2,"offline":{},"temperature":{},"history":[]}"#);
        assert!(matches!(
            result,
            Err(AlertReportError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn garbage_is_a_json_error() {
        assert!(matches!(
            decode(b"not json"),
            Err(AlertReportError::Json(_))
        ));
    }
}
