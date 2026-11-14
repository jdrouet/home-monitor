use std::collections::BTreeMap;

use domain::{
    Measurement, Metric, OFFLINE_AFTER, RoomName, SourceId, SourceKind, Thermometer, Timestamp,
};

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

/// Outdoor weather. The forecast fields are empty unless issued today.
#[derive(Debug, Clone, PartialEq)]
pub struct OutdoorOverview {
    pub reading: ClimateReading,
    pub temperature_min: Option<f64>,
    pub temperature_max: Option<f64>,
    pub summary: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    pub outdoor: OutdoorOverview,
    /// One entry per configured thermometer, in configuration order.
    pub rooms: Vec<RoomOverview>,
    /// BLE sources that reported but are not configured, sorted by source id.
    pub unknown_sensors: Vec<SensorOverview>,
}

/// WMO weather interpretation codes, grouped by the summary shown for them.
const WMO_SUMMARIES: &[(&[u8], &str)] = &[
    (&[0], "Clear sky"),
    (&[1], "Mainly clear"),
    (&[2], "Partly cloudy"),
    (&[3], "Overcast"),
    (&[45, 48], "Fog"),
    (&[51, 53, 55], "Drizzle"),
    (&[56, 57], "Freezing drizzle"),
    (&[61, 63, 65], "Rain"),
    (&[66, 67], "Freezing rain"),
    (&[71, 73, 75], "Snow"),
    (&[77], "Snow grains"),
    (&[80, 81, 82], "Rain showers"),
    (&[85, 86], "Snow showers"),
    (&[95], "Thunderstorm"),
    (&[96, 99], "Thunderstorm with hail"),
];

/// `today_start` is the start of the current local day: forecasts issued
/// before it are hidden.
pub fn build_overview(
    thermometers: &[Thermometer],
    measurements: &[Measurement],
    now: Timestamp,
    today_start: Timestamp,
) -> Overview {
    let weather: Vec<&Measurement> = measurements
        .iter()
        .filter(|m| m.source_kind == SourceKind::Weather)
        .collect();
    let outdoor = outdoor_overview(&weather, now, today_start);
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
        outdoor,
        rooms,
        unknown_sensors,
    }
}

fn outdoor_overview(
    weather: &[&Measurement],
    now: Timestamp,
    today_start: Timestamp,
) -> OutdoorOverview {
    let forecast = |metric| {
        weather
            .iter()
            .find(|m| m.metric == metric && m.timestamp >= today_start)
            .map(|m| m.value)
    };
    OutdoorOverview {
        reading: climate_reading(weather, now),
        temperature_min: forecast(Metric::TemperatureMin),
        temperature_max: forecast(Metric::TemperatureMax),
        summary: forecast(Metric::WeatherCode).and_then(summary_of),
    }
}

fn summary_of(code: f64) -> Option<&'static str> {
    WMO_SUMMARIES
        .iter()
        .find(|(codes, _)| codes.iter().any(|&known| f64::from(known) == code))
        .map(|&(_, summary)| summary)
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
    use domain::{
        MacAddress, Measurement, Metric, RoomName, SourceId, SourceKind, Thermometer, Timestamp,
    };

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
        overview_since(measurements, 0)
    }

    fn overview_since(measurements: &[Measurement], today_start_micros: i64) -> super::Overview {
        build_overview(
            &thermometers(),
            measurements,
            Timestamp::from_micros(NOW_MICROS),
            Timestamp::from_micros(today_start_micros),
        )
    }

    fn weather(metric: Metric, value: f64, age_secs: i64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(NOW_MICROS - age_secs * 1_000_000),
            source_kind: SourceKind::Weather,
            source_id: SourceId::from("open-meteo".to_owned()),
            metric,
            value,
        }
    }

    fn weather_report(age_secs: i64, code: f64) -> Vec<Measurement> {
        vec![
            weather(Metric::Temperature, 4.8, age_secs),
            weather(Metric::Humidity, 78.0, age_secs),
            weather(Metric::TemperatureMin, 2.4, age_secs),
            weather(Metric::TemperatureMax, 11.0, age_secs),
            weather(Metric::WeatherCode, code, age_secs),
        ]
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

    #[test]
    fn fresh_weather_fills_every_field() {
        let outdoor = overview_of(&weather_report(60, 3.0)).outdoor;
        assert_eq!(outdoor.reading.temperature, Some(4.8));
        assert_eq!(outdoor.reading.humidity, Some(78.0));
        assert_eq!(outdoor.reading.freshness, Freshness::Reporting);
        assert_eq!(outdoor.temperature_min, Some(2.4));
        assert_eq!(outdoor.temperature_max, Some(11.0));
        assert_eq!(outdoor.summary, Some("Overcast"));
    }

    #[test]
    fn hour_old_weather_is_not_reporting() {
        let outdoor = overview_of(&weather_report(3600, 3.0)).outdoor;
        assert_eq!(outdoor.reading.freshness, Freshness::NotReporting);
    }

    #[test]
    fn no_weather_is_not_reporting_with_empty_fields() {
        let outdoor = overview_of(&[]).outdoor;
        assert_eq!(outdoor.reading.freshness, Freshness::NotReporting);
        assert_eq!(outdoor.reading.temperature, None);
        assert_eq!(outdoor.temperature_min, None);
        assert_eq!(outdoor.temperature_max, None);
        assert_eq!(outdoor.summary, None);
    }

    #[test]
    fn weather_is_never_an_unknown_sensor() {
        assert!(
            overview_of(&weather_report(60, 3.0))
                .unknown_sensors
                .is_empty()
        );
    }

    #[test]
    fn weather_codes_map_to_summaries() {
        let summary = |code| overview_of(&weather_report(60, code)).outdoor.summary;
        assert_eq!(summary(3.0), Some("Overcast"));
        assert_eq!(summary(99.0), Some("Thunderstorm with hail"));
        assert_eq!(summary(42.0), None);
    }

    #[test]
    fn forecast_issued_before_today_is_hidden_while_current_reading_stays() {
        let today_start = NOW_MICROS - 30 * 1_000_000;
        let outdoor = overview_since(&weather_report(60, 3.0), today_start).outdoor;
        assert_eq!(outdoor.reading.temperature, Some(4.8));
        assert_eq!(outdoor.reading.freshness, Freshness::Reporting);
        assert_eq!(outdoor.temperature_min, None);
        assert_eq!(outdoor.temperature_max, None);
        assert_eq!(outdoor.summary, None);
    }

    #[test]
    fn forecast_issued_after_today_start_is_shown() {
        let today_start = NOW_MICROS - 120 * 1_000_000;
        let outdoor = overview_since(&weather_report(60, 3.0), today_start).outdoor;
        assert_eq!(outdoor.temperature_min, Some(2.4));
        assert_eq!(outdoor.temperature_max, Some(11.0));
        assert_eq!(outdoor.summary, Some("Overcast"));
    }

    #[test]
    fn failed_polls_across_midnight_hide_yesterdays_forecast() {
        // Last poll 23:50, now 00:10: still reporting, but the forecast is yesterday's.
        let today_start = NOW_MICROS - 10 * 60 * 1_000_000;
        let outdoor = overview_since(&weather_report(20 * 60, 3.0), today_start).outdoor;
        assert_eq!(outdoor.reading.freshness, Freshness::Reporting);
        assert_eq!(outdoor.temperature_max, None);
    }
}
