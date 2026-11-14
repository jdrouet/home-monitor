use domain::Timestamp;
use serde::Deserialize;
use use_cases::WeatherReport;

use crate::OpenMeteoError;

#[derive(Deserialize)]
struct Forecast {
    current: Current,
    daily: Daily,
}

#[derive(Deserialize)]
struct Current {
    time: i64,
    temperature_2m: f64,
    relative_humidity_2m: f64,
}

#[derive(Deserialize)]
struct Daily {
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    weather_code: Vec<u8>,
}

pub(crate) fn parse(body: &[u8]) -> Result<WeatherReport, OpenMeteoError> {
    let Forecast { current, daily } = serde_json::from_slice(body)?;
    let (Some(&temperature_max), Some(&temperature_min), Some(&weather_code)) = (
        daily.temperature_2m_max.first(),
        daily.temperature_2m_min.first(),
        daily.weather_code.first(),
    ) else {
        return Err(OpenMeteoError::MissingForecast);
    };
    Ok(WeatherReport {
        observed_at: Timestamp::from_micros(current.time.saturating_mul(1_000_000)),
        temperature: current.temperature_2m,
        humidity: current.relative_humidity_2m,
        temperature_min,
        temperature_max,
        weather_code,
    })
}

#[cfg(test)]
mod tests {
    use use_cases::WeatherReport;

    use crate::OpenMeteoError;

    use super::parse;

    const FIXTURE: &str = include_str!("../fixtures/forecast.json");

    #[test]
    fn fixture_becomes_a_report() {
        let report = parse(FIXTURE.as_bytes()).expect("fixture parses");
        assert_eq!(
            report,
            WeatherReport {
                observed_at: domain::Timestamp::from_micros(1_794_693_600_000_000),
                temperature: 4.8,
                humidity: 78.0,
                temperature_min: 2.4,
                temperature_max: 11.0,
                weather_code: 3,
            }
        );
    }

    #[test]
    fn empty_daily_arrays_are_missing_forecast() {
        let body = FIXTURE
            .replace("[11.0]", "[]")
            .replace("[2.4]", "[]")
            .replace("\"weather_code\":[3]", "\"weather_code\":[]");
        assert!(matches!(
            parse(body.as_bytes()),
            Err(OpenMeteoError::MissingForecast)
        ));
    }

    #[test]
    fn invalid_bodies_are_json_errors() {
        let null_current = FIXTURE.replace(
            "{\"time\":1794693600,\"interval\":900,\"temperature_2m\":4.8,\"relative_humidity_2m\":78}",
            "null",
        );
        for body in [null_current.as_str(), "garbage"] {
            assert!(matches!(
                parse(body.as_bytes()),
                Err(OpenMeteoError::Json(_))
            ));
        }
    }
}
