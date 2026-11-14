use std::collections::BTreeMap;
use std::time::Duration;

use domain::{Measurement, Metric, RoomName, SourceId, SourceKind, Thermometer, Timestamp};

/// A sensor counts as not reporting once its newest measurement is this old.
pub const OFFLINE_AFTER: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Reporting,
    NotReporting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClimateReading {
    pub temperature: Option<f64>,
    pub humidity: Option<f64>,
    pub freshness: Freshness,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoomOverview {
    pub room: RoomName,
    pub reading: ClimateReading,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensorOverview {
    pub source_id: SourceId,
    pub reading: ClimateReading,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    /// One entry per configured thermometer, in configuration order.
    pub rooms: Vec<RoomOverview>,
    /// BLE sources that reported but are not configured, sorted by source id.
    pub unknown_sensors: Vec<SensorOverview>,
}

pub fn build_overview(
    thermometers: &[Thermometer],
    measurements: &[Measurement],
    now: Timestamp,
) -> Overview {
    let mut by_source: BTreeMap<SourceId, Vec<&Measurement>> = BTreeMap::new();
    for measurement in measurements
        .iter()
        .filter(|m| m.source_kind == SourceKind::Ble)
    {
        by_source
            .entry(measurement.source_id.clone())
            .or_default()
            .push(measurement);
    }
    let rooms = thermometers
        .iter()
        .map(|thermometer| {
            let own = by_source
                .remove(&SourceId::from(thermometer.mac))
                .unwrap_or_default();
            RoomOverview {
                room: thermometer.room.clone(),
                reading: climate_reading(&own, now),
            }
        })
        .collect();
    let unknown_sensors = by_source
        .into_iter()
        .map(|(source_id, own)| SensorOverview {
            source_id,
            reading: climate_reading(&own, now),
        })
        .collect();
    Overview {
        rooms,
        unknown_sensors,
    }
}

fn climate_reading(own: &[&Measurement], now: Timestamp) -> ClimateReading {
    let newest = own.iter().map(|m| m.timestamp).max();
    let freshness = match newest {
        Some(at) if now.duration_since(at) < OFFLINE_AFTER => Freshness::Reporting,
        _ => Freshness::NotReporting,
    };
    ClimateReading {
        temperature: value_of(own, Metric::Temperature),
        humidity: value_of(own, Metric::Humidity),
        freshness,
    }
}

fn value_of(own: &[&Measurement], metric: Metric) -> Option<f64> {
    own.iter().find(|m| m.metric == metric).map(|m| m.value)
}

#[cfg(test)]
mod tests {
    use domain::{MacAddress, Measurement, Metric, RoomName, SourceKind, Thermometer, Timestamp};

    use super::{Freshness, build_overview};

    const MAC_BEDROOM: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 1]);
    const MAC_OFFICE: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 2]);
    const MAC_STRANGER: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9]);
    const NOW_MICROS: i64 = 10_000_000_000;

    fn thermometers() -> Vec<Thermometer> {
        [(MAC_BEDROOM, "bedroom"), (MAC_OFFICE, "office")]
            .into_iter()
            .map(|(mac, room)| Thermometer {
                mac,
                room: RoomName::try_from(room.to_owned()).expect("valid room"),
            })
            .collect()
    }

    fn measurement(mac: MacAddress, metric: Metric, value: f64, age_secs: i64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(NOW_MICROS - age_secs * 1_000_000),
            source_kind: SourceKind::Ble,
            source_id: mac.into(),
            metric,
            value,
        }
    }

    fn overview_of(measurements: &[Measurement]) -> super::Overview {
        build_overview(
            &thermometers(),
            measurements,
            Timestamp::from_micros(NOW_MICROS),
        )
    }

    #[test]
    fn fresh_sensor_is_reporting() {
        let overview = overview_of(&[
            measurement(MAC_BEDROOM, Metric::Temperature, 21.3, 30),
            measurement(MAC_BEDROOM, Metric::Humidity, 51.5, 30),
        ]);
        let bedroom = &overview.rooms[0].reading;
        assert_eq!(bedroom.temperature, Some(21.3));
        assert_eq!(bedroom.humidity, Some(51.5));
        assert_eq!(bedroom.freshness, Freshness::Reporting);
    }

    #[test]
    fn exactly_one_hour_is_not_reporting() {
        let overview = overview_of(&[measurement(MAC_BEDROOM, Metric::Temperature, 21.0, 3600)]);
        assert_eq!(overview.rooms[0].reading.freshness, Freshness::NotReporting);
        assert_eq!(overview.rooms[0].reading.temperature, Some(21.0));
    }

    #[test]
    fn fifty_nine_minutes_is_reporting() {
        let overview = overview_of(&[measurement(MAC_BEDROOM, Metric::Temperature, 21.0, 59 * 60)]);
        assert_eq!(overview.rooms[0].reading.freshness, Freshness::Reporting);
    }

    #[test]
    fn newest_metric_decides_freshness() {
        let overview = overview_of(&[
            measurement(MAC_BEDROOM, Metric::Temperature, 21.0, 7200),
            measurement(MAC_BEDROOM, Metric::Battery, 88.0, 10),
        ]);
        assert_eq!(overview.rooms[0].reading.freshness, Freshness::Reporting);
    }

    #[test]
    fn missing_data_is_not_reporting() {
        let overview = overview_of(&[]);
        let rooms: Vec<_> = overview.rooms.iter().map(|r| r.room.as_str()).collect();
        assert_eq!(rooms, ["bedroom", "office"]);
        let reading = &overview.rooms[1].reading;
        assert_eq!(reading.temperature, None);
        assert_eq!(reading.humidity, None);
        assert_eq!(reading.freshness, Freshness::NotReporting);
        assert!(overview.unknown_sensors.is_empty());
    }

    #[test]
    fn humidity_only_leaves_temperature_empty() {
        let overview = overview_of(&[measurement(MAC_OFFICE, Metric::Humidity, 40.0, 5)]);
        let reading = &overview.rooms[1].reading;
        assert_eq!(reading.temperature, None);
        assert_eq!(reading.humidity, Some(40.0));
    }

    #[test]
    fn unconfigured_sensors_are_listed_sorted() {
        let other = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 5]);
        let overview = overview_of(&[
            measurement(MAC_STRANGER, Metric::Temperature, 18.0, 5),
            measurement(other, Metric::Temperature, 19.0, 5),
            measurement(MAC_BEDROOM, Metric::Temperature, 21.0, 5),
        ]);
        let unknown: Vec<_> = overview
            .unknown_sensors
            .iter()
            .map(|s| s.source_id.as_str())
            .collect();
        assert_eq!(unknown, ["A4:C1:38:00:00:05", "A4:C1:38:00:00:09"]);
        assert_eq!(overview.unknown_sensors[0].reading.temperature, Some(19.0));
    }
}
