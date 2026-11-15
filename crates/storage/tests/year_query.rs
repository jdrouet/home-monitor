use std::env;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use domain::{MacAddress, Measurement, Metric, SeriesQuery, SourceId, SourceKind, Timestamp};
use storage::ParquetStore;
use tempfile::TempDir;
use use_cases::MeasurementStore;

const DAY: i64 = 86_400_000_000;
const WEATHER: u8 = 9;

fn source(id: u8) -> (SourceKind, SourceId) {
    if id == WEATHER {
        (SourceKind::Weather, String::from("open-meteo").into())
    } else {
        (
            SourceKind::Ble,
            MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, id]).into(),
        )
    }
}

fn day_batch(day: i64) -> Vec<Measurement> {
    let mut rows = Vec::new();
    for (id, count, step_secs) in [
        (1, 1440, 60),
        (2, 1440, 60),
        (3, 1440, 60),
        (4, 1440, 60),
        (WEATHER, 96, 900),
    ] {
        let (source_kind, source_id) = source(id);
        for i in 0..count {
            rows.push(Measurement {
                timestamp: Timestamp::from_micros(day * DAY + i * step_secs * 1_000_000),
                source_kind,
                source_id: source_id.clone(),
                metric: Metric::Temperature,
                value: f64::from(id) + day as f64 + (i % 8) as f64 * 0.1,
            });
        }
    }
    rows
}

async fn store_days(dir: &Path, first_day: i64, days: i64) -> ParquetStore {
    let store = ParquetStore::open(dir).expect("open");
    for day in first_day..first_day + days {
        store.append(day_batch(day)).expect("append");
        store
            .flush_due(Timestamp::from_micros((day + 1) * DAY + 1))
            .await
            .expect("flush");
    }
    store
}

fn daily(first_day: i64, days: i64) -> SeriesQuery {
    SeriesQuery {
        metric: Metric::Temperature,
        source_id: None,
        from: Timestamp::from_micros(first_day * DAY),
        to: Timestamp::from_micros((first_day + days) * DAY),
        bucket_secs: NonZeroU32::new(86_400),
    }
}

#[tokio::test]
async fn daily_buckets_average_each_source() {
    let dir = TempDir::new().expect("tempdir");
    let store = store_days(dir.path(), 0, 3).await;
    let rows = store.series(&daily(0, 3)).await.expect("series");

    assert_eq!(rows.len(), 15);
    for row in rows {
        let day = row.timestamp.as_micros() / DAY;
        let id = (1..=4)
            .chain([WEATHER])
            .find(|id| source(*id).1 == row.source_id)
            .expect("known source");
        let expected = f64::from(id) + day as f64 + 0.35;
        assert!((row.value - expected).abs() < 1e-9, "{row:?}");
    }
}

fn median(mut durations: Vec<Duration>) -> Duration {
    durations.sort();
    durations[durations.len() / 2]
}

// Diagnostic timing over the year ending today. `YEAR_DATA_DIR` keeps the
// generated data for other runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "generates a year of data"]
async fn one_year_daily_query_duration() {
    let scratch;
    let dir: PathBuf = match env::var_os("YEAR_DATA_DIR") {
        Some(dir) => dir.into(),
        None => {
            scratch = TempDir::new().expect("tempdir");
            scratch.path().to_owned()
        }
    };
    let today = Timestamp::now().as_micros() / DAY;
    let started = Instant::now();
    drop(store_days(&dir, today - 365, 365).await);
    println!("generation: {:?}", started.elapsed());

    let store = ParquetStore::open(&dir).expect("reopen");
    store.append(day_batch(today)).expect("today");
    let query = daily(today - 365, 366);
    let mut durations = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let rows = store.series(&query).await.expect("series");
        durations.push(started.elapsed());
        assert_eq!(rows.len(), 366 * 5);
    }
    println!("first query: {:?}", durations[0]);
    println!("median query: {:?}", median(durations));
}
