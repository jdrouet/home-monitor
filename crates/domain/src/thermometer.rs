use serde::Deserialize;

use crate::{DomainError, MacAddress, RoomName, TemperatureRange};

/// A configured sensor, the room it sits in and the temperatures that room
/// may have, when it has a limit.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "ThermometerEntry")]
pub struct Thermometer {
    pub mac: MacAddress,
    pub room: RoomName,
    pub temperature_range: Option<TemperatureRange>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThermometerEntry {
    mac: MacAddress,
    room: RoomName,
    min_temperature: Option<f64>,
    max_temperature: Option<f64>,
}

impl TryFrom<ThermometerEntry> for Thermometer {
    type Error = DomainError;

    fn try_from(entry: ThermometerEntry) -> Result<Self, Self::Error> {
        let temperature_range = match (entry.min_temperature, entry.max_temperature) {
            (None, None) => None,
            (Some(min), Some(max)) => Some(TemperatureRange::new(min, max)?),
            _ => return Err(DomainError::IncompleteTemperatureRange),
        };
        Ok(Self {
            mac: entry.mac,
            room: entry.room,
            temperature_range,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{DomainError, TemperatureRange};

    use super::Thermometer;

    const BASE: &str = r#"{"mac":"A4:C1:38:00:00:01","room":"bedroom""#;

    fn parse(extra: &str) -> Result<Thermometer, serde_json::Error> {
        serde_json::from_str(&format!("{BASE}{extra}}}"))
    }

    #[test]
    fn no_limits_means_no_range() {
        assert_eq!(parse("").expect("valid").temperature_range, None);
    }

    #[test]
    fn both_limits_make_a_range() {
        let thermometer =
            parse(r#","min_temperature":18.0,"max_temperature":20.0"#).expect("valid");
        assert_eq!(
            thermometer.temperature_range,
            Some(TemperatureRange::new(18.0, 20.0).expect("valid"))
        );
    }

    #[test]
    fn one_limit_is_rejected() {
        for extra in [r#","min_temperature":18.0"#, r#","max_temperature":20.0"#] {
            let error = parse(extra).expect_err("incomplete");
            assert!(
                error
                    .to_string()
                    .contains(&DomainError::IncompleteTemperatureRange.to_string())
            );
        }
    }

    #[test]
    fn an_invalid_range_is_rejected() {
        let error =
            parse(r#","min_temperature":20.0,"max_temperature":18.0"#).expect_err("invalid");
        assert!(
            error
                .to_string()
                .contains(&DomainError::InvalidTemperatureRange.to_string())
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(parse(r#","colour":"red""#).is_err());
    }
}
