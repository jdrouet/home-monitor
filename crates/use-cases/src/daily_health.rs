use domain::{Measurement, Metric, SourceId, SourceKind, Timestamp};

/// One day's health figures; a figure the provider lacks is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DailyHealth {
    pub sleep_seconds: Option<u32>,
    pub sleep_score: Option<u32>,
    pub resting_heart_rate: Option<u32>,
    pub steps: Option<u32>,
    pub body_battery_max: Option<u32>,
    pub body_battery_min: Option<u32>,
}

impl DailyHealth {
    /// One measurement per known figure, all stamped `stamp`.
    pub fn measurements(&self, source_id: &SourceId, stamp: Timestamp) -> Vec<Measurement> {
        [
            (Metric::SleepDuration, self.sleep_seconds),
            (Metric::SleepScore, self.sleep_score),
            (Metric::RestingHeartRate, self.resting_heart_rate),
            (Metric::Steps, self.steps),
            (Metric::BodyBatteryMax, self.body_battery_max),
            (Metric::BodyBatteryMin, self.body_battery_min),
        ]
        .into_iter()
        .filter_map(|(metric, value)| {
            value.map(|value| Measurement {
                timestamp: stamp,
                source_kind: SourceKind::Health,
                source_id: source_id.clone(),
                metric,
                value: f64::from(value),
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use domain::{Metric, SourceId, SourceKind, Timestamp};

    use super::DailyHealth;

    fn source_id() -> SourceId {
        SourceId::from("garmin".to_owned())
    }

    #[test]
    fn measurements_cover_every_known_figure_at_the_stamp() {
        let health = DailyHealth {
            sleep_seconds: Some(27_120),
            sleep_score: Some(81),
            resting_heart_rate: Some(52),
            steps: Some(8_431),
            body_battery_max: Some(96),
            body_battery_min: Some(14),
        };
        let batch = health.measurements(&source_id(), Timestamp::from_micros(42));
        let pairs: Vec<_> = batch
            .iter()
            .map(|measurement| (measurement.metric, measurement.value))
            .collect();
        assert_eq!(
            pairs,
            [
                (Metric::SleepDuration, 27_120.0),
                (Metric::SleepScore, 81.0),
                (Metric::RestingHeartRate, 52.0),
                (Metric::Steps, 8_431.0),
                (Metric::BodyBatteryMax, 96.0),
                (Metric::BodyBatteryMin, 14.0),
            ]
        );
        for measurement in &batch {
            assert_eq!(measurement.timestamp, Timestamp::from_micros(42));
            assert_eq!(measurement.source_kind, SourceKind::Health);
            assert_eq!(measurement.source_id.as_str(), "garmin");
        }
    }

    #[test]
    fn missing_figures_are_not_published() {
        let health = DailyHealth {
            steps: Some(10),
            ..DailyHealth::default()
        };
        let batch = health.measurements(&source_id(), Timestamp::from_micros(1));
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].metric, Metric::Steps);
        assert!(
            DailyHealth::default()
                .measurements(&source_id(), Timestamp::from_micros(1))
                .is_empty()
        );
    }
}
