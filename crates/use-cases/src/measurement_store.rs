use domain::{Measurement, SeriesQuery, Timestamp};

/// Durable storage of measurements and queries over them.
pub trait MeasurementStore {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Returns only once the batch is durable; the caller acks after `Ok`.
    fn append(&self, batch: Vec<Measurement>) -> Result<(), Self::Error>;

    fn latest(&self) -> Vec<Measurement>;

    fn series(
        &self,
        query: &SeriesQuery,
    ) -> impl Future<Output = Result<Vec<Measurement>, Self::Error>> + Send;

    /// Moves every day before the one containing `now` into long-term storage.
    fn flush_due(&self, now: Timestamp) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
