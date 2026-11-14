use std::fs;

use domain::{MacAddress, Measurement, Metric, SeriesQuery, SourceKind, Timestamp};
use storage::ParquetStore;
use tempfile::TempDir;
use use_cases::MeasurementStore;

const DAY: i64 = 86_400_000_000;

fn reading(metric: Metric, mac_last: u8, micros: i64, value: f64) -> Measurement {
    Measurement {
        timestamp: Timestamp::from_micros(micros),
        source_kind: SourceKind::Ble,
        source_id: MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, mac_last]).into(),
        metric,
        value,
    }
}

fn temperature(mac_last: u8, micros: i64, value: f64) -> Measurement {
    reading(Metric::Temperature, mac_last, micros, value)
}

fn range(from: i64, to: i64) -> SeriesQuery {
    SeriesQuery {
        metric: Metric::Temperature,
        source_id: None,
        from: Timestamp::from_micros(from),
        to: Timestamp::from_micros(to),
        bucket_secs: None,
    }
}

fn first_day_batch() -> Vec<Measurement> {
    vec![
        temperature(1, 10, 20.0),
        temperature(1, 20, 21.0),
        temperature(0xEE, 30, 18.0),
        reading(Metric::Humidity, 1, 10, 50.0),
    ]
}

#[tokio::test]
async fn readings_are_logged_flushed_and_queried_once() {
    let dir = TempDir::new().expect("tempdir");
    let now = Timestamp::from_micros(2 * DAY + 5);

    let store = ParquetStore::open(dir.path()).expect("open");
    store.append(first_day_batch()).expect("first day");
    store
        .append(vec![temperature(1, DAY + 10, 22.0)])
        .expect("second day");
    store
        .append(vec![temperature(1, 2 * DAY + 1, 23.0)])
        .expect("today");
    store.append(first_day_batch()).expect("same batch again");
    drop(store);

    let store = ParquetStore::open(dir.path()).expect("reopen");
    store.flush_due(now).await.expect("flush");
    let root = dir.path();
    assert!(
        root.join("metric=temperature/date=1970-01-01/data.parquet")
            .exists()
    );
    assert!(
        root.join("metric=humidity/date=1970-01-01/data.parquet")
            .exists()
    );
    assert!(
        root.join("metric=temperature/date=1970-01-02/data.parquet")
            .exists()
    );
    assert!(!root.join("wal/1970-01-01.ndjson").exists());
    assert!(root.join("wal/1970-01-03.ndjson").exists());

    let first_day = store.series(&range(0, DAY)).await.expect("first day");
    assert_eq!(
        first_day,
        [
            temperature(1, 10, 20.0),
            temperature(1, 20, 21.0),
            temperature(0xEE, 30, 18.0)
        ]
    );
    let today = store.series(&range(2 * DAY, 3 * DAY)).await.expect("today");
    assert_eq!(today, [temperature(1, 2 * DAY + 1, 23.0)]);
}

#[tokio::test]
async fn a_redelivered_flushed_batch_changes_nothing() {
    let dir = TempDir::new().expect("tempdir");
    let store = ParquetStore::open(dir.path()).expect("open");
    store.append(first_day_batch()).expect("append");
    store
        .flush_due(Timestamp::from_micros(DAY + 5))
        .await
        .expect("flush");
    let latest = store.latest();

    let redelivered = first_day_batch()
        .into_iter()
        .map(|row| Measurement { value: 99.0, ..row })
        .collect();
    store.append(redelivered).expect("redelivery");

    assert_eq!(store.series(&range(0, DAY)).await.expect("series").len(), 3);
    assert_eq!(store.latest(), latest);
    assert!(!dir.path().join("wal/1970-01-01.ndjson").exists());
}

#[tokio::test]
async fn a_query_reads_only_the_days_and_metric_it_needs() {
    let dir = TempDir::new().expect("tempdir");
    let store = ParquetStore::open(dir.path()).expect("open");
    store.append(first_day_batch()).expect("first day");
    store
        .append(vec![temperature(1, DAY + 10, 22.0)])
        .expect("second day");
    store
        .flush_due(Timestamp::from_micros(2 * DAY + 5))
        .await
        .expect("flush");

    for unrelated in [
        "metric=temperature/date=1970-01-02/data.parquet",
        "metric=humidity/date=1970-01-01/data.parquet",
    ] {
        fs::write(dir.path().join(unrelated), b"not parquet").expect("overwrite");
    }

    let first_day = store.series(&range(0, DAY)).await;
    assert_eq!(first_day.expect("first day only").len(), 3);
    assert!(store.series(&range(0, 2 * DAY)).await.is_err());
}
