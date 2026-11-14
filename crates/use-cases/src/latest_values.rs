use std::collections::HashMap;
use std::collections::hash_map::Entry;

use domain::{Measurement, Metric, SourceId, SourceKind};

/// The newest measurement per source and metric.
#[derive(Default)]
pub struct LatestValues(HashMap<(SourceKind, SourceId, Metric), Measurement>);

impl LatestValues {
    /// Replaces a stored measurement only with a strictly newer one, so
    /// redelivered, reordered or repeated-timestamp batches change nothing.
    pub fn record(&mut self, measurements: Vec<Measurement>) {
        for measurement in measurements {
            let key = (
                measurement.source_kind,
                measurement.source_id.clone(),
                measurement.metric,
            );
            match self.0.entry(key) {
                Entry::Occupied(mut stored) if measurement.timestamp > stored.get().timestamp => {
                    stored.insert(measurement);
                }
                Entry::Occupied(_) => {}
                Entry::Vacant(slot) => {
                    slot.insert(measurement);
                }
            }
        }
    }

    /// All stored measurements, ordered by source id, then metric.
    pub fn snapshot(&self) -> Vec<Measurement> {
        let mut all: Vec<Measurement> = self.0.values().cloned().collect();
        all.sort_by(|a, b| {
            (&a.source_id, a.metric, a.source_kind).cmp(&(&b.source_id, b.metric, b.source_kind))
        });
        all
    }
}

#[cfg(test)]
mod tests {
    use domain::{MacAddress, Measurement, Metric, SourceKind, Timestamp};

    use super::LatestValues;

    fn measurement(last_octet: u8, metric: Metric, value: f64, micros: i64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([0, 0, 0, 0, 0, last_octet]).into(),
            metric,
            value,
        }
    }

    #[test]
    fn newer_replaces_older() {
        let mut latest = LatestValues::default();
        latest.record(vec![measurement(1, Metric::Temperature, 20.0, 10)]);
        latest.record(vec![measurement(1, Metric::Temperature, 21.0, 20)]);
        assert_eq!(
            latest.snapshot(),
            [measurement(1, Metric::Temperature, 21.0, 20)]
        );
    }

    #[test]
    fn older_is_ignored() {
        let mut latest = LatestValues::default();
        latest.record(vec![measurement(1, Metric::Temperature, 21.0, 20)]);
        latest.record(vec![measurement(1, Metric::Temperature, 20.0, 10)]);
        assert_eq!(
            latest.snapshot(),
            [measurement(1, Metric::Temperature, 21.0, 20)]
        );
    }

    #[test]
    fn same_timestamp_keeps_the_first_value() {
        let mut latest = LatestValues::default();
        latest.record(vec![measurement(1, Metric::Temperature, 20.0, 10)]);
        latest.record(vec![measurement(1, Metric::Temperature, 99.0, 10)]);
        assert_eq!(
            latest.snapshot(),
            [measurement(1, Metric::Temperature, 20.0, 10)]
        );
    }

    #[test]
    fn redelivery_is_idempotent() {
        let mut latest = LatestValues::default();
        let batch = vec![measurement(1, Metric::Humidity, 50.0, 10)];
        latest.record(batch.clone());
        latest.record(batch.clone());
        assert_eq!(latest.snapshot(), batch);
    }

    #[test]
    fn snapshot_is_ordered_by_source_then_metric() {
        let mut latest = LatestValues::default();
        latest.record(vec![
            measurement(2, Metric::Temperature, 1.0, 1),
            measurement(1, Metric::Humidity, 2.0, 1),
            measurement(1, Metric::Temperature, 3.0, 1),
        ]);
        let order: Vec<_> = latest
            .snapshot()
            .iter()
            .map(|m| (m.source_id.as_str().to_owned(), m.metric))
            .collect();
        assert_eq!(
            order,
            [
                ("00:00:00:00:00:01".to_owned(), Metric::Temperature),
                ("00:00:00:00:00:01".to_owned(), Metric::Humidity),
                ("00:00:00:00:00:02".to_owned(), Metric::Temperature),
            ]
        );
    }
}
