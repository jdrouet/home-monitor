use serde::{Deserialize, Serialize};

use crate::{BatchError, Measurement};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct WireOut<'a> {
    version: u32,
    measurements: &'a [Measurement],
}

#[derive(Deserialize)]
struct WireIn {
    version: u32,
    measurements: Vec<Measurement>,
}

pub fn encode(measurements: &[Measurement]) -> Vec<u8> {
    serde_json::to_vec(&WireOut {
        version: SCHEMA_VERSION,
        measurements,
    })
    .expect("a measurement batch always serializes")
}

pub fn decode(payload: &[u8]) -> Result<Vec<Measurement>, BatchError> {
    let wire: WireIn = serde_json::from_slice(payload)?;
    if wire.version != SCHEMA_VERSION {
        return Err(BatchError::UnsupportedVersion(wire.version));
    }
    Ok(wire.measurements)
}

#[cfg(test)]
mod tests {
    use crate::{BatchError, MacAddress, Measurement, Metric, SourceKind, Timestamp};

    use super::{decode, encode};

    fn sample() -> Measurement {
        let address: MacAddress = "A4:C1:38:1C:02:76".parse().expect("valid address");
        Measurement {
            timestamp: Timestamp::from_micros(1_700_000_000_000_000),
            source_kind: SourceKind::Ble,
            source_id: address.into(),
            metric: Metric::Temperature,
            value: 21.27,
        }
    }

    #[test]
    fn encoding_matches_the_wire_format() {
        let expected = r#"{"version":1,"measurements":[{"timestamp":1700000000000000,"source_kind":"ble","source_id":"A4:C1:38:1C:02:76","metric":"temperature","value":21.27}]}"#;
        assert_eq!(encode(&[sample()]), expected.as_bytes());
    }

    #[test]
    fn round_trips() {
        let batch = vec![sample()];
        assert_eq!(decode(&encode(&batch)).expect("decodes"), batch);
    }

    #[test]
    fn other_versions_are_rejected() {
        let result = decode(br#"{"version":2,"measurements":[]}"#);
        assert!(matches!(result, Err(BatchError::UnsupportedVersion(2))));
    }

    #[test]
    fn garbage_is_a_json_error() {
        assert!(matches!(decode(b"not json"), Err(BatchError::Json(_))));
    }
}
