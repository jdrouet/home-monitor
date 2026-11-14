use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use domain::Measurement;
use tracing::warn;

use crate::StorageError;
use crate::reading_key::ReadingKey;
use crate::sync_dir::sync_dir;
use crate::utc_date::utc_date;

const SEGMENT_EXTENSION: &str = "ndjson";

/// An append-only log with one NDJSON segment per UTC date of the reading's
/// timestamp, plus the unflushed rows and their keys in memory.
pub(crate) struct Wal {
    dir: PathBuf,
    days: BTreeMap<NaiveDate, Vec<Measurement>>,
    keys: HashSet<ReadingKey>,
}

impl Wal {
    /// Opens the log under `data_dir`, repairing a torn last line in each
    /// segment. A segment with any other corrupt line is an error.
    pub(crate) fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let dir = data_dir.join("wal");
        fs::create_dir_all(&dir)?;
        sync_dir(&dir)?;
        sync_dir(data_dir)?;
        let mut wal = Self {
            dir,
            days: BTreeMap::new(),
            keys: HashSet::new(),
        };
        for entry in fs::read_dir(&wal.dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == SEGMENT_EXTENSION) {
                wal.load(recover_segment(&path)?);
            }
        }
        Ok(wal)
    }

    /// Logs and fsyncs the readings not seen before, returning them. Readings
    /// outside the dates the log can name are dropped.
    pub(crate) fn append(
        &mut self,
        batch: Vec<Measurement>,
    ) -> Result<Vec<Measurement>, StorageError> {
        let fresh = self.fresh_by_day(batch);
        for (date, rows) in &fresh {
            self.write_segment(*date, rows)?;
        }
        let logged = fresh.into_values().flatten().collect::<Vec<_>>();
        self.load(logged.clone());
        Ok(logged)
    }

    /// Days with a segment, strictly before `today`.
    pub(crate) fn due_days(&self, today: NaiveDate) -> Vec<NaiveDate> {
        self.days.range(..today).map(|(date, _)| *date).collect()
    }

    pub(crate) fn day(&self, date: NaiveDate) -> &[Measurement] {
        self.days.get(&date).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn rows(&self) -> impl Iterator<Item = &Measurement> {
        self.days.values().flatten()
    }

    /// Deletes the segment of `date` and forgets its rows.
    pub(crate) fn remove_day(&mut self, date: NaiveDate) -> Result<(), StorageError> {
        fs::remove_file(self.segment_path(date))?;
        sync_dir(&self.dir)?;
        for row in self.days.remove(&date).unwrap_or_default() {
            self.keys.remove(&ReadingKey::from(&row));
        }
        Ok(())
    }

    fn segment_path(&self, date: NaiveDate) -> PathBuf {
        self.dir.join(format!("{date}.{SEGMENT_EXTENSION}"))
    }

    fn fresh_by_day(&self, batch: Vec<Measurement>) -> BTreeMap<NaiveDate, Vec<Measurement>> {
        let mut seen = HashSet::new();
        let mut by_day: BTreeMap<NaiveDate, Vec<Measurement>> = BTreeMap::new();
        for row in batch {
            let Some(date) = utc_date(row.timestamp) else {
                warn!(
                    timestamp = row.timestamp.as_micros(),
                    "dropping reading outside years 1 to 9999"
                );
                continue;
            };
            let key = ReadingKey::from(&row);
            if !self.keys.contains(&key) && seen.insert(key) {
                by_day.entry(date).or_default().push(row);
            }
        }
        by_day
    }

    fn write_segment(&self, date: NaiveDate, rows: &[Measurement]) -> Result<(), StorageError> {
        let path = self.segment_path(date);
        let created = !path.exists();
        let mut lines = Vec::new();
        for row in rows {
            serde_json::to_writer(&mut lines, row).map_err(io::Error::from)?;
            lines.push(b'\n');
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(&lines)?;
        file.sync_data()?;
        if created {
            sync_dir(&self.dir)?;
        }
        Ok(())
    }

    fn load(&mut self, rows: Vec<Measurement>) {
        for row in rows {
            if let Some(date) = utc_date(row.timestamp)
                && self.keys.insert(ReadingKey::from(&row))
            {
                self.days.entry(date).or_default().push(row);
            }
        }
    }
}

/// Cuts a torn last line (a crash mid-append) and fsyncs the file, so rows
/// are never trusted for deduplication before they are durable.
fn recover_segment(path: &Path) -> Result<Vec<Measurement>, StorageError> {
    let bytes = fs::read(path)?;
    let valid_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let file = OpenOptions::new().write(true).open(path)?;
    if valid_len < bytes.len() {
        file.set_len(u64::try_from(valid_len).expect("file length fits u64"))?;
    }
    file.sync_data()?;
    bytes[..valid_len]
        .split_inclusive(|byte| *byte == b'\n')
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_slice(line).map_err(|source| StorageError::CorruptLog {
                path: path.to_owned(),
                line: index + 1,
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use domain::{MacAddress, Measurement, Metric, SourceKind, Timestamp};
    use tempfile::TempDir;

    use super::{Wal, recover_segment};
    use crate::StorageError;

    const DAY: i64 = 86_400_000_000;

    fn reading(micros: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([1, 2, 3, 4, 5, 6]).into(),
            metric: Metric::Temperature,
            value,
        }
    }

    fn line(measurement: &Measurement) -> String {
        format!(
            "{}\n",
            serde_json::to_string(measurement).expect("serializes")
        )
    }

    fn segment(dir: &TempDir, name: &str) -> std::path::PathBuf {
        dir.path().join("wal").join(name)
    }

    fn rows(wal: &Wal) -> Vec<Measurement> {
        wal.rows().cloned().collect()
    }

    #[test]
    fn appended_rows_survive_a_reopen() {
        let dir = TempDir::new().expect("tempdir");
        let batch = vec![reading(1, 1.0), reading(2, 2.0)];
        let mut wal = Wal::open(dir.path()).expect("open");
        assert_eq!(wal.append(batch.clone()).expect("append"), batch);
        drop(wal);
        assert_eq!(rows(&Wal::open(dir.path()).expect("reopen")), batch);
    }

    #[test]
    fn a_repeated_batch_is_logged_once_even_across_a_reopen() {
        let dir = TempDir::new().expect("tempdir");
        let batch = vec![reading(1, 1.0), reading(2, 2.0)];
        let mut wal = Wal::open(dir.path()).expect("open");
        wal.append(batch.clone()).expect("append");
        assert!(wal.append(batch.clone()).expect("again").is_empty());
        drop(wal);
        let mut wal = Wal::open(dir.path()).expect("reopen");
        assert!(wal.append(batch).expect("after reopen").is_empty());
        let text = fs::read_to_string(segment(&dir, "1970-01-01.ndjson")).expect("segment");
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn a_duplicate_inside_one_batch_is_logged_once() {
        let dir = TempDir::new().expect("tempdir");
        let mut wal = Wal::open(dir.path()).expect("open");
        let logged = wal
            .append(vec![reading(1, 1.0), reading(1, 9.0)])
            .expect("append");
        assert_eq!(logged, [reading(1, 1.0)]);
    }

    #[test]
    fn a_batch_spanning_midnight_fills_two_segments() {
        let dir = TempDir::new().expect("tempdir");
        let mut wal = Wal::open(dir.path()).expect("open");
        wal.append(vec![reading(DAY - 1, 1.0), reading(DAY, 2.0)])
            .expect("append");
        assert!(segment(&dir, "1970-01-01.ndjson").exists());
        assert!(segment(&dir, "1970-01-02.ndjson").exists());
        let today = chrono::NaiveDate::from_ymd_opt(1970, 1, 2).expect("date");
        assert_eq!(wal.due_days(today).len(), 1);
        assert_eq!(wal.day(today), [reading(DAY, 2.0)]);
    }

    #[test]
    fn a_torn_tail_is_cut_and_the_next_append_parses() {
        let dir = TempDir::new().expect("tempdir");
        drop(Wal::open(dir.path()).expect("create"));
        let path = segment(&dir, "1970-01-01.ndjson");
        let valid = line(&reading(1, 1.0));
        fs::write(&path, format!("{valid}{{\"timest")).expect("write");
        let mut wal = Wal::open(dir.path()).expect("open");
        assert_eq!(fs::read_to_string(&path).expect("read"), valid);
        wal.append(vec![reading(2, 2.0)]).expect("append");
        drop(wal);
        assert_eq!(
            rows(&Wal::open(dir.path()).expect("reopen")),
            [reading(1, 1.0), reading(2, 2.0)]
        );
    }

    #[test]
    fn recover_segment_returns_the_valid_rows_and_ends_with_a_newline() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("1970-01-01.ndjson");
        fs::write(&path, format!("{}{{\"timest", line(&reading(1, 1.0)))).expect("write");
        assert_eq!(recover_segment(&path).expect("recover"), [reading(1, 1.0)]);
        assert!(fs::read(&path).expect("read").ends_with(b"\n"));
    }

    #[test]
    fn a_corrupt_middle_line_refuses_to_open() {
        let dir = TempDir::new().expect("tempdir");
        drop(Wal::open(dir.path()).expect("create"));
        let path = segment(&dir, "1970-01-01.ndjson");
        fs::write(
            &path,
            format!(
                "{}garbage\n{}",
                line(&reading(1, 1.0)),
                line(&reading(2, 2.0))
            ),
        )
        .expect("write");
        let Err(StorageError::CorruptLog {
            path: bad, line, ..
        }) = Wal::open(dir.path())
        else {
            panic!("expected a corrupt log error");
        };
        assert_eq!((bad, line), (path, 2));
    }

    #[test]
    fn readings_outside_the_nameable_years_are_dropped() {
        let dir = TempDir::new().expect("tempdir");
        let mut wal = Wal::open(dir.path()).expect("open");
        assert!(
            wal.append(vec![reading(i64::MAX, 1.0)])
                .expect("append")
                .is_empty()
        );
        assert_eq!(
            fs::read_dir(dir.path().join("wal")).expect("dir").count(),
            0
        );
    }

    #[test]
    fn remove_day_deletes_the_segment_and_frees_the_keys() {
        let dir = TempDir::new().expect("tempdir");
        let mut wal = Wal::open(dir.path()).expect("open");
        wal.append(vec![reading(1, 1.0)]).expect("append");
        let day = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("date");
        wal.remove_day(day).expect("remove");
        assert!(!segment(&dir, "1970-01-01.ndjson").exists());
        assert!(wal.day(day).is_empty());
        assert_eq!(wal.append(vec![reading(1, 1.0)]).expect("append").len(), 1);
    }
}
