use serde::{Deserialize, Serialize};

/// A measured quantity. Units are implied: degrees Celsius, percent relative
/// humidity, percent battery charge and a WMO weather code. The forecast
/// minimum and maximum are today's, in degrees Celsius.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Temperature,
    Humidity,
    Battery,
    TemperatureMin,
    TemperatureMax,
    WeatherCode,
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
        ] {
            let json = serde_json::to_string(&metric).expect("metric serializes");
            assert_eq!(json, format!("\"{}\"", metric.as_str()));
        }
    }
}
