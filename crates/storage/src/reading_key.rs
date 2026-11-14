use domain::{Measurement, Metric, SourceId, Timestamp};

/// What makes a reading unique: the same sensor never reports one metric twice
/// at the same instant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ReadingKey {
    timestamp: Timestamp,
    source_id: SourceId,
    metric: Metric,
}

impl From<&Measurement> for ReadingKey {
    fn from(measurement: &Measurement) -> Self {
        Self {
            timestamp: measurement.timestamp,
            source_id: measurement.source_id.clone(),
            metric: measurement.metric,
        }
    }
}
