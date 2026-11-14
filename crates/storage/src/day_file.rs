use std::collections::BTreeMap;
use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use datafusion::parquet::arrow::ArrowWriter;
use datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use datafusion::parquet::basic::{Compression, ZstdLevel};
use datafusion::parquet::file::properties::WriterProperties;
use domain::{Measurement, Metric};

use crate::StorageError;
use crate::measurement_batch::{from_batch, schema, to_batch};
use crate::replace_file::replace_file;

/// The day file of `metric` on `date`, laid out as hive partitions.
pub(crate) fn path(data_dir: &Path, metric: Metric, date: NaiveDate) -> PathBuf {
    data_dir
        .join(format!("metric={}", metric.as_str()))
        .join(format!("date={date}"))
        .join("data.parquet")
}

/// All rows of a day file, or none when the file does not exist.
pub(crate) fn read(
    data_dir: &Path,
    metric: Metric,
    date: NaiveDate,
) -> Result<Vec<Measurement>, StorageError> {
    let file = match File::open(path(data_dir, metric, date)) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut rows = Vec::new();
    for batch in ParquetRecordBatchReaderBuilder::try_new(file)?.build()? {
        rows.extend(from_batch(&batch?, metric)?);
    }
    Ok(rows)
}

/// Rewrites the day file with its rows plus `rows`, sorted by timestamp then
/// source. When both hold a reading for the same instant and source, the
/// existing one wins.
pub(crate) fn merge(
    data_dir: &Path,
    metric: Metric,
    date: NaiveDate,
    rows: &[Measurement],
) -> Result<(), StorageError> {
    let mut merged = BTreeMap::new();
    for row in read(data_dir, metric, date)?
        .into_iter()
        .chain(rows.iter().cloned())
    {
        merged
            .entry((row.timestamp, row.source_id.clone()))
            .or_insert(row);
    }
    let batch = to_batch(&merged.into_values().collect::<Vec<_>>())?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .build();
    replace_file(data_dir, &path(data_dir, metric, date), |file| {
        let mut writer = ArrowWriter::try_new(file, schema(), Some(properties))?;
        writer.write(&batch)?;
        writer.close()?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::NaiveDate;
    use domain::{MacAddress, Measurement, Metric, SourceKind, Timestamp};
    use tempfile::TempDir;

    use super::{merge, path, read};

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(1970, 1, 1).expect("date")
    }

    fn reading(micros: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([1, 2, 3, 4, 5, 6]).into(),
            metric: Metric::Temperature,
            value,
        }
    }

    #[test]
    fn a_missing_file_reads_as_empty() {
        let dir = TempDir::new().expect("tempdir");
        assert!(
            read(dir.path(), Metric::Temperature, day())
                .expect("read")
                .is_empty()
        );
    }

    #[test]
    fn merging_into_nothing_writes_sorted_rows_under_the_hive_path() {
        let dir = TempDir::new().expect("tempdir");
        merge(
            dir.path(),
            Metric::Temperature,
            day(),
            &[reading(2, 2.0), reading(1, 1.0)],
        )
        .expect("merge");
        assert!(
            dir.path()
                .join("metric=temperature/date=1970-01-01/data.parquet")
                .exists()
        );
        assert_eq!(
            read(dir.path(), Metric::Temperature, day()).expect("read"),
            [reading(1, 1.0), reading(2, 2.0)]
        );
    }

    #[test]
    fn merging_twice_keeps_each_reading_once_and_the_existing_value() {
        let dir = TempDir::new().expect("tempdir");
        merge(
            dir.path(),
            Metric::Temperature,
            day(),
            &[reading(1, 1.0), reading(2, 2.0)],
        )
        .expect("first");
        merge(
            dir.path(),
            Metric::Temperature,
            day(),
            &[reading(2, 99.0), reading(3, 3.0)],
        )
        .expect("second");
        assert_eq!(
            read(dir.path(), Metric::Temperature, day()).expect("read"),
            [reading(1, 1.0), reading(2, 2.0), reading(3, 3.0)]
        );
    }

    #[test]
    fn a_stale_temp_file_is_replaced_and_not_left_behind() {
        let dir = TempDir::new().expect("tempdir");
        let file = path(dir.path(), Metric::Temperature, day());
        fs::create_dir_all(file.parent().expect("parent")).expect("dirs");
        fs::write(file.with_extension("parquet.tmp"), b"garbage").expect("stale");
        merge(dir.path(), Metric::Temperature, day(), &[reading(1, 1.0)]).expect("merge");
        assert_eq!(
            fs::read_dir(file.parent().expect("parent"))
                .expect("dir")
                .count(),
            1
        );
        assert_eq!(
            read(dir.path(), Metric::Temperature, day())
                .expect("read")
                .len(),
            1
        );
    }
}
