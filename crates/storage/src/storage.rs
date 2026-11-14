use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use domain::{Measurement, Metric, SeriesQuery, Timestamp, batch};
use use_cases::LatestValues;

use crate::StorageError;
use crate::day_file;
use crate::reading_key::ReadingKey;
use crate::replace_file::replace_file;
use crate::sync_dir::sync_dir;
use crate::utc_date::{clamped_utc_date, utc_date};
use crate::wal::Wal;

const SNAPSHOT_FILE: &str = "latest.json";

/// The write path: a log of unflushed readings, day files for older days, and
/// the latest value per sensor.
pub(crate) struct Storage {
    data_dir: PathBuf,
    wal: Wal,
    latest: LatestValues,
}

impl Storage {
    /// Opens `data_dir`, creating it if needed. The latest values come from the
    /// snapshot of the last flush plus every row still in the log.
    pub(crate) fn open(data_dir: &Path) -> Result<Self, StorageError> {
        fs::create_dir_all(data_dir)?;
        let data_dir = fs::canonicalize(data_dir)?;
        if let Some(parent) = data_dir.parent() {
            sync_dir(parent)?;
        }
        let wal = Wal::open(&data_dir)?;
        let mut latest = LatestValues::default();
        latest.record(read_snapshot(&data_dir)?);
        latest.record(wal.rows().cloned().collect());
        Ok(Self {
            data_dir,
            wal,
            latest,
        })
    }

    /// Returns once the new readings are fsynced in the log. Readings already
    /// stored, in the log or in a day file, are dropped.
    pub(crate) fn append(&mut self, batch: Vec<Measurement>) -> Result<(), StorageError> {
        let fresh = self.drop_flushed(batch)?;
        self.latest.record(self.wal.append(fresh)?);
        Ok(())
    }

    /// Writes every log day before the one containing `now` to its day files.
    /// A segment is deleted only after all its day files are durable, and a
    /// snapshot of the latest values is durable before the first deletion.
    pub(crate) fn flush_due(&mut self, now: Timestamp) -> Result<(), StorageError> {
        let due = self.wal.due_days(clamped_utc_date(now));
        if due.is_empty() {
            return Ok(());
        }
        self.write_snapshot()?;
        for date in due {
            self.flush_day(date)?;
        }
        Ok(())
    }

    /// Unflushed readings that match `query`, before any bucketing.
    pub(crate) fn unflushed(&self, query: &SeriesQuery) -> Vec<Measurement> {
        self.wal
            .rows()
            .filter(|row| {
                row.metric == query.metric
                    && (query.from..query.to).contains(&row.timestamp)
                    && query
                        .source_id
                        .as_ref()
                        .is_none_or(|id| *id == row.source_id)
            })
            .cloned()
            .collect()
    }

    pub(crate) fn latest(&self) -> Vec<Measurement> {
        self.latest.snapshot()
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn drop_flushed(&self, batch: Vec<Measurement>) -> Result<Vec<Measurement>, StorageError> {
        let mut groups: BTreeMap<(Metric, Option<NaiveDate>), Vec<Measurement>> = BTreeMap::new();
        for row in batch {
            groups
                .entry((row.metric, utc_date(row.timestamp)))
                .or_default()
                .push(row);
        }
        let mut kept = Vec::new();
        for ((metric, date), rows) in groups {
            let Some(date) = date else {
                kept.extend(rows);
                continue;
            };
            let stored: HashSet<ReadingKey> = day_file::read(&self.data_dir, metric, date)?
                .iter()
                .map(ReadingKey::from)
                .collect();
            kept.extend(
                rows.into_iter()
                    .filter(|row| !stored.contains(&ReadingKey::from(row))),
            );
        }
        Ok(kept)
    }

    fn write_snapshot(&self) -> Result<(), StorageError> {
        let bytes = batch::encode(&self.latest.snapshot());
        replace_file(&self.data_dir, &self.data_dir.join(SNAPSHOT_FILE), |file| {
            Ok(file.write_all(&bytes)?)
        })
    }

    fn flush_day(&mut self, date: NaiveDate) -> Result<(), StorageError> {
        let mut by_metric: BTreeMap<Metric, Vec<Measurement>> = BTreeMap::new();
        for row in self.wal.day(date) {
            by_metric.entry(row.metric).or_default().push(row.clone());
        }
        for (metric, rows) in &by_metric {
            day_file::merge(&self.data_dir, *metric, date, rows)?;
        }
        self.wal.remove_day(date)
    }
}

fn read_snapshot(data_dir: &Path) -> Result<Vec<Measurement>, StorageError> {
    match fs::read(data_dir.join(SNAPSHOT_FILE)) {
        Ok(bytes) => Ok(batch::decode(&bytes)?),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::NaiveDate;
    use domain::{MacAddress, Measurement, Metric, SeriesQuery, SourceKind, Timestamp};
    use tempfile::TempDir;

    use super::Storage;
    use crate::day_file;

    const DAY: i64 = 86_400_000_000;

    fn d() -> NaiveDate {
        NaiveDate::from_ymd_opt(1970, 1, 1).expect("date")
    }

    fn now() -> Timestamp {
        Timestamp::from_micros(DAY + 5)
    }

    fn reading(metric: Metric, micros: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([1, 2, 3, 4, 5, 6]).into(),
            metric,
            value,
        }
    }

    fn temperature(micros: i64, value: f64) -> Measurement {
        reading(Metric::Temperature, micros, value)
    }

    fn humidity(micros: i64, value: f64) -> Measurement {
        reading(Metric::Humidity, micros, value)
    }

    fn day_rows(dir: &TempDir, metric: Metric) -> Vec<Measurement> {
        day_file::read(dir.path(), metric, d()).expect("read day file")
    }

    fn segment_exists(dir: &TempDir) -> bool {
        dir.path().join("wal/1970-01-01.ndjson").exists()
    }

    fn two_metrics() -> Vec<Measurement> {
        vec![
            temperature(1, 20.0),
            temperature(2, 21.0),
            humidity(1, 50.0),
        ]
    }

    fn open(dir: &TempDir) -> Storage {
        Storage::open(dir.path()).expect("open")
    }

    #[test]
    fn crash_while_writing_a_day_file() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(two_metrics()).expect("append");
        let file = day_file::path(storage.data_dir(), Metric::Temperature, d());
        fs::create_dir_all(file.parent().expect("parent")).expect("dirs");
        fs::write(file.with_extension("parquet.tmp"), b"half a fil").expect("partial");
        drop(storage);

        let mut storage = open(&dir);
        storage.flush_due(now()).expect("flush");
        assert_eq!(
            day_rows(&dir, Metric::Temperature),
            [temperature(1, 20.0), temperature(2, 21.0)]
        );
        assert_eq!(day_rows(&dir, Metric::Humidity), [humidity(1, 50.0)]);
        assert!(!segment_exists(&dir));
    }

    #[test]
    fn crash_after_one_metric_was_written() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(two_metrics()).expect("append");
        day_file::merge(
            storage.data_dir(),
            Metric::Temperature,
            d(),
            &two_metrics()[..2],
        )
        .expect("first metric");
        drop(storage);

        let mut storage = open(&dir);
        storage.flush_due(now()).expect("flush");
        assert_eq!(day_rows(&dir, Metric::Temperature).len(), 2);
        assert_eq!(day_rows(&dir, Metric::Humidity).len(), 1);
        assert!(!segment_exists(&dir));
    }

    #[test]
    fn crash_after_all_day_files_before_log_removal() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(two_metrics()).expect("append");
        let rows = two_metrics();
        day_file::merge(storage.data_dir(), Metric::Temperature, d(), &rows[..2]).expect("t");
        day_file::merge(storage.data_dir(), Metric::Humidity, d(), &rows[2..]).expect("h");
        drop(storage);

        let mut storage = open(&dir);
        storage.flush_due(now()).expect("flush");
        assert_eq!(day_rows(&dir, Metric::Temperature).len(), 2);
        assert_eq!(day_rows(&dir, Metric::Humidity).len(), 1);
        assert!(!segment_exists(&dir));
    }

    #[test]
    fn crash_after_snapshot_before_day_files() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(two_metrics()).expect("append");
        storage.write_snapshot().expect("snapshot");
        drop(storage);

        let mut storage = open(&dir);
        assert_eq!(storage.latest().len(), 2);
        storage.flush_due(now()).expect("flush");
        assert_eq!(day_rows(&dir, Metric::Temperature).len(), 2);
    }

    #[test]
    fn restart_after_flush_keeps_latest() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(two_metrics()).expect("append");
        let before = storage.latest();
        storage.flush_due(now()).expect("flush");
        drop(storage);

        assert!(!segment_exists(&dir));
        assert_eq!(open(&dir).latest(), before);
        assert_eq!(before, [temperature(2, 21.0), humidity(1, 50.0)]);
    }

    #[test]
    fn flush_is_idempotent() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(two_metrics()).expect("append");
        storage.flush_due(now()).expect("first");
        storage.flush_due(now()).expect("second");
        assert_eq!(day_rows(&dir, Metric::Temperature).len(), 2);
    }

    #[test]
    fn late_reading_for_a_flushed_day() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(vec![temperature(1, 20.0)]).expect("append");
        storage.flush_due(now()).expect("flush");

        storage
            .append(vec![temperature(1, 99.0), temperature(3, 23.0)])
            .expect("late");
        let segment = fs::read_to_string(dir.path().join("wal/1970-01-01.ndjson")).expect("log");
        assert_eq!(segment.lines().count(), 1);
        storage.flush_due(now()).expect("second flush");
        assert_eq!(
            day_rows(&dir, Metric::Temperature),
            [temperature(1, 20.0), temperature(3, 23.0)]
        );
    }

    #[test]
    fn redelivered_reading_after_its_day_was_flushed_is_dropped() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage.append(vec![temperature(1, 20.0)]).expect("append");
        storage.flush_due(now()).expect("flush");

        storage
            .append(vec![temperature(1, 99.0)])
            .expect("redelivery");
        assert!(!segment_exists(&dir));
        assert_eq!(day_rows(&dir, Metric::Temperature), [temperature(1, 20.0)]);
        assert_eq!(storage.latest(), [temperature(1, 20.0)]);
    }

    #[test]
    fn drop_flushed_passes_rows_without_a_day_file() {
        let dir = TempDir::new().expect("tempdir");
        let storage = open(&dir);
        let batch = vec![temperature(1, 20.0), temperature(i64::MAX, 1.0)];
        let kept = storage.drop_flushed(batch.clone()).expect("filter");
        assert_eq!(kept.len(), 2);
        assert!(batch.iter().all(|row| kept.contains(row)));
    }

    #[test]
    fn unflushed_filters_by_metric_range_and_source() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        let other_source = Measurement {
            source_id: MacAddress::new([9, 9, 9, 9, 9, 9]).into(),
            ..temperature(2, 5.0)
        };
        storage
            .append(vec![
                temperature(1, 1.0),
                temperature(2, 2.0),
                temperature(3, 3.0),
                humidity(2, 50.0),
                other_source.clone(),
            ])
            .expect("append");
        let all = SeriesQuery {
            metric: Metric::Temperature,
            source_id: None,
            from: Timestamp::from_micros(2),
            to: Timestamp::from_micros(3),
            bucket_secs: None,
        };
        let mut rows = storage.unflushed(&all);
        rows.sort_by(|a, b| a.value.total_cmp(&b.value));
        assert_eq!(rows, [temperature(2, 2.0), other_source.clone()]);
        let one = SeriesQuery {
            source_id: Some(temperature(0, 0.0).source_id),
            ..all
        };
        assert_eq!(storage.unflushed(&one), [temperature(2, 2.0)]);
    }

    #[test]
    fn today_is_not_flushed() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        storage
            .append(vec![temperature(DAY + 1, 20.0)])
            .expect("append");
        storage.flush_due(now()).expect("flush");
        assert!(dir.path().join("wal/1970-01-02.ndjson").exists());
        assert!(!dir.path().join("metric=temperature").exists());
    }

    #[test]
    fn unknown_sensor_is_kept_under_its_mac() {
        let dir = TempDir::new().expect("tempdir");
        let mut storage = open(&dir);
        let stranger = Measurement {
            source_id: MacAddress::new([0xDE, 0xAD, 0xBE, 0xEF, 0, 1]).into(),
            ..temperature(1, 18.0)
        };
        storage.append(vec![stranger.clone()]).expect("append");
        storage.flush_due(now()).expect("flush");
        assert_eq!(
            day_rows(&dir, Metric::Temperature),
            std::slice::from_ref(&stranger)
        );
        assert_eq!(open(&dir).latest(), [stranger]);
    }
}
