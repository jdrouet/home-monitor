use domain::{Measurement, Metric, SourceId, SourceKind, Timestamp};

/// Current outdoor conditions and today's forecast, as issued at `observed_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherReport {
    pub observed_at: Timestamp,
    pub temperature: f64,
    pub humidity: f64,
    pub temperature_min: f64,
    pub temperature_max: f64,
    pub weather_code: u8,
}

impl WeatherReport {
    /// One measurement per metric, all stamped `observed_at`.
    pub fn measurements(&self, source_id: &SourceId) -> Vec<Measurement> {
        [
            (Metric::Temperature, self.temperature),
            (Metric::Humidity, self.humidity),
            (Metric::TemperatureMin, self.temperature_min),
            (Metric::TemperatureMax, self.temperature_max),
            (Metric::WeatherCode, f64::from(self.weather_code)),
        ]
        .into_iter()
        .map(|(metric, value)| Measurement {
            timestamp: self.observed_at,
            source_kind: SourceKind::Weather,
            source_id: source_id.clone(),
            metric,
            value,
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use domain::{Metric, SourceId, SourceKind, Timestamp};

    use super::WeatherReport;

    #[test]
    fn measurements_cover_every_metric_at_the_observation_time() {
        let report = WeatherReport {
            observed_at: Timestamp::from_micros(42),
            temperature: 4.8,
            humidity: 78.0,
            temperature_min: 2.4,
            temperature_max: 11.0,
            weather_code: 3,
        };
        let measurements = report.measurements(&SourceId::from("open-meteo".to_owned()));
        let values: Vec<_> = measurements.iter().map(|m| (m.metric, m.value)).collect();
        assert_eq!(
            values,
            [
                (Metric::Temperature, 4.8),
                (Metric::Humidity, 78.0),
                (Metric::TemperatureMin, 2.4),
                (Metric::TemperatureMax, 11.0),
                (Metric::WeatherCode, 3.0),
            ]
        );
        assert!(measurements.iter().all(|m| {
            m.timestamp == Timestamp::from_micros(42)
                && m.source_kind == SourceKind::Weather
                && m.source_id.as_str() == "open-meteo"
        }));
    }
}
