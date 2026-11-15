use serde::{Deserialize, Serialize};

/// A measured quantity. Units are implied: degrees Celsius, percent relative
/// humidity, percent battery charge and a WMO weather code. The forecast
/// minimum and maximum are today's, in degrees Celsius. The daily health
/// metrics are sleep duration in seconds, a sleep score from 0 to 100, resting
/// heart rate in beats per minute, a step count and Body Battery levels from 0
/// to 100. `SessionValid` is 1 when the Garmin session worked and 0 when a
/// login is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Temperature,
    Humidity,
    Battery,
    TemperatureMin,
    TemperatureMax,
    WeatherCode,
    SleepDuration,
    SleepScore,
    RestingHeartRate,
    Steps,
    BodyBatteryMax,
    BodyBatteryMin,
    SessionValid,
}

impl Metric {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::Humidity => "humidity",
            Self::Battery => "battery",
            Self::TemperatureMin => "temperature_min",
            Self::TemperatureMax => "temperature_max",
            Self::WeatherCode => "weather_code",
            Self::SleepDuration => "sleep_duration",
            Self::SleepScore => "sleep_score",
            Self::RestingHeartRate => "resting_heart_rate",
            Self::Steps => "steps",
            Self::BodyBatteryMax => "body_battery_max",
            Self::BodyBatteryMin => "body_battery_min",
            Self::SessionValid => "session_valid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Metric;

    #[test]
    fn as_str_matches_the_serde_name() {
        for metric in [
            Metric::Temperature,
            Metric::Humidity,
            Metric::Battery,
            Metric::TemperatureMin,
            Metric::TemperatureMax,
            Metric::WeatherCode,
            Metric::SleepDuration,
            Metric::SleepScore,
            Metric::RestingHeartRate,
            Metric::Steps,
            Metric::BodyBatteryMax,
            Metric::BodyBatteryMin,
            Metric::SessionValid,
        ] {
            let json = serde_json::to_string(&metric).expect("metric serializes");
            assert_eq!(json, format!("\"{}\"", metric.as_str()));
        }
    }
}
