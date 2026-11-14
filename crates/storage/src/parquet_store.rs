use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use domain::{Measurement, SeriesQuery, Timestamp};
use tokio::sync::RwLock;
use use_cases::MeasurementStore;

use crate::StorageError;
use crate::query_series::query_series;
use crate::storage::Storage;

/// Measurements in a write-ahead log and Parquet day files under one directory.
///
/// Appends and flushes do blocking file I/O on the caller's thread.
pub struct ParquetStore {
    core: Mutex<Storage>,
    // Queries hold the read side and flushes the write side, so a day is never
    // read from its day file and its log segment at once.
    flush_gate: RwLock<()>,
    data_dir: PathBuf,
}

impl ParquetStore {
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let core = Storage::open(data_dir)?;
        let data_dir = core.data_dir().to_owned();
        Ok(Self {
            core: Mutex::new(core),
            flush_gate: RwLock::new(()),
            data_dir,
        })
    }

    fn core(&self) -> MutexGuard<'_, Storage> {
        self.core.lock().expect("storage lock poisoned")
    }
}

impl MeasurementStore for ParquetStore {
    type Error = StorageError;

    fn append(&self, batch: Vec<Measurement>) -> Result<(), StorageError> {
        self.core().append(batch)
    }

    fn latest(&self) -> Vec<Measurement> {
        self.core().latest()
    }

    async fn series(&self, query: &SeriesQuery) -> Result<Vec<Measurement>, StorageError> {
        let _reading = self.flush_gate.read().await;
        let unflushed = self.core().unflushed(query);
        query_series(&self.data_dir, query, unflushed).await
    }

    async fn flush_due(&self, now: Timestamp) -> Result<(), StorageError> {
        let _flushing = self.flush_gate.write().await;
        self.core().flush_due(now)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use domain::{MacAddress, Measurement, Metric, SeriesQuery, SourceKind, Timestamp};
    use tempfile::TempDir;
    use use_cases::MeasurementStore;

    use super::ParquetStore;

    const DAY: i64 = 86_400_000_000;

    fn temperature(micros: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([1, 2, 3, 4, 5, 6]).into(),
            metric: Metric::Temperature,
            value,
        }
    }

    fn everything() -> SeriesQuery {
        SeriesQuery {
            metric: Metric::Temperature,
            source_id: None,
            from: Timestamp::from_micros(0),
            to: Timestamp::from_micros(10 * DAY),
            bucket_secs: None,
        }
    }

    fn assert_send<T: Send>(_: T) {}

    #[tokio::test]
    async fn appended_readings_are_served_by_series_and_latest() {
        let dir = TempDir::new().expect("tempdir");
        let store = ParquetStore::open(dir.path()).expect("open");
        store
            .append(vec![temperature(1, 1.0), temperature(2, 2.0)])
            .expect("append");
        let rows = store.series(&everything()).await.expect("series");
        assert_eq!(rows, [temperature(1, 1.0), temperature(2, 2.0)]);
        assert_eq!(store.latest(), [temperature(2, 2.0)]);
    }

    #[tokio::test]
    async fn flushed_and_unflushed_readings_are_each_served_once() {
        let dir = TempDir::new().expect("tempdir");
        let store = ParquetStore::open(dir.path()).expect("open");
        store.append(vec![temperature(1, 1.0)]).expect("yesterday");
        store
            .flush_due(Timestamp::from_micros(DAY + 5))
            .await
            .expect("flush");
        store
            .append(vec![temperature(DAY + 1, 2.0), temperature(1, 99.0)])
            .expect("today");
        let rows = store.series(&everything()).await.expect("series");
        assert_eq!(rows, [temperature(1, 1.0), temperature(DAY + 1, 2.0)]);
    }

    #[tokio::test]
    async fn a_source_id_is_kept_exactly_as_stored() {
        let dir = TempDir::new().expect("tempdir");
        let store = ParquetStore::open(dir.path()).expect("open");
        let odd = Measurement {
            source_id: String::from("a4:c1:38:1c:02:76").into(),
            ..temperature(1, 1.0)
        };
        store.append(vec![odd.clone()]).expect("append");
        store
            .flush_due(Timestamp::from_micros(DAY + 5))
            .await
            .expect("flush");
        store.append(vec![odd.clone()]).expect("redelivery");
        let rows = store.series(&everything()).await.expect("series");
        assert_eq!(rows, [odd]);
    }

    #[tokio::test]
    async fn buckets_apply_to_the_merged_rows() {
        let dir = TempDir::new().expect("tempdir");
        let store = ParquetStore::open(dir.path()).expect("open");
        store
            .append(vec![temperature(1, 10.0), temperature(2, 20.0)])
            .expect("append");
        let query = SeriesQuery {
            bucket_secs: NonZeroU32::new(60),
            ..everything()
        };
        let rows = store.series(&query).await.expect("series");
        assert_eq!(rows, [temperature(0, 15.0)]);
    }

    #[tokio::test]
    async fn the_futures_can_cross_threads() {
        let dir = TempDir::new().expect("tempdir");
        let store = ParquetStore::open(dir.path()).expect("open");
        let query = everything();
        assert_send(store.series(&query));
        assert_send(store.flush_due(Timestamp::from_micros(0)));
    }
}
