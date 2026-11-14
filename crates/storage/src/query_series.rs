use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, TimeUnit};
use datafusion::common::ScalarValue;
use datafusion::datasource::file_format::parquet::ParquetFormat;
use datafusion::datasource::listing::{
    ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl,
};
use datafusion::functions_aggregate::expr_fn::avg;
use datafusion::logical_expr::{Expr, cast};
use datafusion::prelude::{DataFrame, SessionContext, col, lit};
use domain::{Measurement, SeriesQuery, Timestamp};

use crate::StorageError;
use crate::measurement_batch::{from_batch, schema, to_batch};
use crate::utc_date::clamped_utc_date;

const COLUMNS: [&str; 4] = ["timestamp", "source_kind", "source_id", "value"];

/// Reads the day files that can hold `query` (pruned by their metric and date
/// path) and adds the `unflushed` rows, which the caller already filtered.
/// Rows come sorted by timestamp, then source.
pub(crate) async fn query_series(
    data_dir: &Path,
    query: &SeriesQuery,
    unflushed: Vec<Measurement>,
) -> Result<Vec<Measurement>, StorageError> {
    if query.from >= query.to {
        return Ok(Vec::new());
    }
    let context = SessionContext::new();
    let recent = context.read_batch(to_batch(&unflushed)?)?;
    let all = stored_rows(&context, data_dir, query)?.union(recent)?;
    let batches = bucketed(all, query.bucket_secs)?
        .sort(vec![
            col("timestamp").sort(true, true),
            col("source_id").sort(true, true),
        ])?
        .collect()
        .await?;
    let mut rows = Vec::new();
    for batch in &batches {
        rows.extend(from_batch(batch, query.metric)?);
    }
    Ok(rows)
}

fn stored_rows(
    context: &SessionContext,
    data_dir: &Path,
    query: &SeriesQuery,
) -> Result<DataFrame, StorageError> {
    let format = ParquetFormat::default().with_force_view_types(false);
    let options = ListingOptions::new(Arc::new(format))
        .with_file_extension(".parquet")
        .with_table_partition_cols(vec![
            ("metric".to_owned(), DataType::Utf8),
            ("date".to_owned(), DataType::Utf8),
        ]);
    let url = ListingTableUrl::parse(format!("{}/", data_dir.display()))?;
    let config = ListingTableConfig::new(url)
        .with_listing_options(options)
        .with_schema(schema());
    let table = ListingTable::try_new(config)?;
    // read_table takes an `Arc<dyn TableProvider>`: the library API requires it.
    let mut frame = context.read_table(Arc::new(table))?.filter(window(query))?;
    if let Some(source_id) = &query.source_id {
        frame = frame.filter(col("source_id").eq(lit(source_id.as_str())))?;
    }
    Ok(frame.select_columns(&COLUMNS)?)
}

fn window(query: &SeriesQuery) -> Expr {
    let last_instant = Timestamp::from_micros(query.to.as_micros().saturating_sub(1));
    col("metric")
        .eq(lit(query.metric.as_str()))
        .and(col("date").gt_eq(lit(clamped_utc_date(query.from).to_string())))
        .and(col("date").lt_eq(lit(clamped_utc_date(last_instant).to_string())))
        .and(col("timestamp").gt_eq(instant(query.from)))
        .and(col("timestamp").lt(instant(query.to)))
}

fn instant(timestamp: Timestamp) -> Expr {
    lit(ScalarValue::TimestampMicrosecond(
        Some(timestamp.as_micros()),
        Some("UTC".into()),
    ))
}

fn bucketed(frame: DataFrame, bucket_secs: Option<NonZeroU32>) -> Result<DataFrame, StorageError> {
    let Some(secs) = bucket_secs else {
        return Ok(frame);
    };
    let width = i64::from(secs.get()) * 1_000_000;
    let micros = cast(col("timestamp"), DataType::Int64);
    // Floor, not truncate: SQL `%` keeps the sign of the dividend.
    let floored = micros.clone() - ((micros % lit(width)) + lit(width)) % lit(width);
    let start = cast(
        floored,
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
    );
    Ok(frame.with_column("timestamp", start)?.aggregate(
        vec![col("source_kind"), col("source_id"), col("timestamp")],
        vec![avg(col("value")).alias("value")],
    )?)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::num::NonZeroU32;

    use domain::{MacAddress, Measurement, Metric, SeriesQuery, SourceId, SourceKind, Timestamp};
    use tempfile::TempDir;

    use super::query_series;
    use crate::day_file;

    const SECOND: i64 = 1_000_000;

    fn day() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("date")
    }

    fn mac(last: u8) -> SourceId {
        MacAddress::new([0, 0, 0, 0, 0, last]).into()
    }

    fn reading(metric: Metric, source: u8, micros: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: mac(source),
            metric,
            value,
        }
    }

    fn query(from: i64, to: i64) -> SeriesQuery {
        SeriesQuery {
            metric: Metric::Temperature,
            source_id: None,
            from: Timestamp::from_micros(from),
            to: Timestamp::from_micros(to),
            bucket_secs: None,
        }
    }

    fn store(dir: &TempDir, rows: &[Measurement]) {
        day_file::merge(dir.path(), rows[0].metric, day(), rows).expect("merge");
    }

    fn temperature(source: u8, micros: i64, value: f64) -> Measurement {
        reading(Metric::Temperature, source, micros, value)
    }

    #[tokio::test]
    async fn an_empty_directory_gives_no_rows() {
        let dir = TempDir::new().expect("tempdir");
        let rows = query_series(dir.path(), &query(0, SECOND), Vec::new()).await;
        assert!(rows.expect("query").is_empty());
    }

    #[tokio::test]
    async fn the_log_and_latest_snapshot_do_not_disturb_the_listing() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join("wal")).expect("wal");
        fs::write(dir.path().join("wal/1970-01-01.ndjson"), b"{}\n").expect("segment");
        fs::write(dir.path().join("latest.json"), b"{}").expect("snapshot");
        store(&dir, &[temperature(1, 1, 1.0)]);
        let rows = query_series(dir.path(), &query(0, SECOND), Vec::new()).await;
        assert_eq!(rows.expect("query"), [temperature(1, 1, 1.0)]);
    }

    #[tokio::test]
    async fn stored_and_unflushed_rows_come_back_once_and_sorted() {
        let dir = TempDir::new().expect("tempdir");
        store(&dir, &[temperature(1, 3, 3.0), temperature(1, 1, 1.0)]);
        let rows = query_series(dir.path(), &query(0, SECOND), vec![temperature(1, 2, 2.0)]).await;
        assert_eq!(
            rows.expect("query"),
            [
                temperature(1, 1, 1.0),
                temperature(1, 2, 2.0),
                temperature(1, 3, 3.0)
            ]
        );
    }

    #[tokio::test]
    async fn from_is_inclusive_and_to_is_exclusive() {
        let dir = TempDir::new().expect("tempdir");
        store(
            &dir,
            &[
                temperature(1, 1, 1.0),
                temperature(1, 2, 2.0),
                temperature(1, 3, 3.0),
            ],
        );
        let rows = query_series(dir.path(), &query(2, 3), Vec::new()).await;
        assert_eq!(rows.expect("query"), [temperature(1, 2, 2.0)]);
    }

    #[tokio::test]
    async fn an_empty_or_inverted_range_gives_no_rows() {
        let dir = TempDir::new().expect("tempdir");
        store(&dir, &[temperature(1, 1, 1.0)]);
        for (from, to) in [(1, 1), (2, 1)] {
            let rows = query_series(dir.path(), &query(from, to), Vec::new()).await;
            assert!(rows.expect("query").is_empty());
        }
    }

    #[tokio::test]
    async fn source_id_filters_stored_rows() {
        let dir = TempDir::new().expect("tempdir");
        store(&dir, &[temperature(1, 1, 1.0), temperature(2, 1, 2.0)]);
        let wanted = SeriesQuery {
            source_id: Some(mac(2)),
            ..query(0, SECOND)
        };
        let rows = query_series(dir.path(), &wanted, Vec::new()).await;
        assert_eq!(rows.expect("query"), [temperature(2, 1, 2.0)]);
    }

    #[tokio::test]
    async fn other_metrics_are_excluded() {
        let dir = TempDir::new().expect("tempdir");
        store(&dir, &[temperature(1, 1, 1.0)]);
        store(&dir, &[reading(Metric::Humidity, 1, 1, 50.0)]);
        let rows = query_series(dir.path(), &query(0, SECOND), Vec::new()).await;
        assert_eq!(rows.expect("query"), [temperature(1, 1, 1.0)]);
    }

    #[tokio::test]
    async fn buckets_average_per_source_and_carry_their_start() {
        let dir = TempDir::new().expect("tempdir");
        store(
            &dir,
            &[
                temperature(1, 10 * SECOND, 10.0),
                temperature(1, 50 * SECOND, 20.0),
                temperature(1, 70 * SECOND, 30.0),
            ],
        );
        let bucketed = SeriesQuery {
            bucket_secs: NonZeroU32::new(60),
            ..query(0, 120 * SECOND)
        };
        let rows = query_series(dir.path(), &bucketed, Vec::new()).await;
        assert_eq!(
            rows.expect("query"),
            [temperature(1, 0, 15.0), temperature(1, 60 * SECOND, 30.0)]
        );
    }
}
