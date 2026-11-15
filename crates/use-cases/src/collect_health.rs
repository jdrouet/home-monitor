use chrono::NaiveDate;
use domain::{SourceId, SourceKind};
use tracing::{error, warn};

use crate::{HealthDay, HealthProvider, Publisher};

/// How a pull ended. The alerter raises the login alert on `SessionFailed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthPull {
    /// The pull ran to the end. `fetched` lists the days whose fetch worked
    /// and whose publish worked or had nothing to publish.
    Completed {
        fetched: Vec<NaiveDate>,
    },
    SessionFailed,
    /// Garmin asked the client to slow down.
    Throttled,
    /// The session could not open for a reason a login would not fix, such
    /// as a network error or an outage.
    Failed,
}

/// Pulls each of `days` through one session and publishes the measurements.
/// A failed day is logged and skipped; a failed session or a throttled fetch
/// ends the pull.
pub async fn collect_health<H: HealthProvider, P: Publisher>(
    provider: &H,
    publisher: &P,
    source_id: &SourceId,
    days: &[HealthDay],
) -> HealthPull {
    let session = match provider.open().await {
        Ok(session) => session,
        Err(error) if H::is_throttled(&error) => {
            warn!(%error, "health session throttled, skipping this pull");
            return HealthPull::Throttled;
        }
        Err(error) if H::is_session_error(&error) => {
            error!(%error, "health session failed, skipping this pull");
            return HealthPull::SessionFailed;
        }
        Err(error) => {
            error!(%error, "health session could not open, skipping this pull");
            return HealthPull::Failed;
        }
    };
    let mut fetched = Vec::new();
    for day in days {
        let health = match provider.daily(&session, day.date).await {
            Ok(health) => health,
            Err(error) if H::is_session_error(&error) => {
                error!(%error, "health session rejected, skipping this pull");
                return HealthPull::SessionFailed;
            }
            Err(error) if H::is_throttled(&error) => {
                warn!(%error, "health fetch throttled, ending this pull");
                return HealthPull::Throttled;
            }
            Err(error) => {
                warn!(%error, date = %day.date, "health fetch failed, skipping this day");
                continue;
            }
        };
        let batch = health.measurements(source_id, day.stamp);
        if batch.is_empty() {
            fetched.push(day.date);
            continue;
        }
        match publisher
            .publish(SourceKind::Health, source_id, &batch)
            .await
        {
            Ok(()) => fetched.push(day.date),
            Err(error) => {
                warn!(%error, date = %day.date, "health publish failed, skipping this day");
            }
        }
    }
    HealthPull::Completed { fetched }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fmt;
    use std::io;
    use std::sync::Mutex;

    use chrono::NaiveDate;
    use domain::{Measurement, SourceId, SourceKind, Timestamp};

    use crate::{DailyHealth, HealthDay, HealthProvider, Publisher};

    use super::{HealthPull, collect_health};

    #[derive(Debug, Clone, Copy)]
    enum TestError {
        Session,
        Day,
        Throttled,
        Network,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test error")
        }
    }

    impl std::error::Error for TestError {}

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, day).expect("valid date")
    }

    fn day(day_of_month: u32) -> HealthDay {
        HealthDay {
            date: date(day_of_month),
            stamp: Timestamp::from_micros(i64::from(day_of_month)),
        }
    }

    fn completed(days: &[u32]) -> HealthPull {
        HealthPull::Completed {
            fetched: days.iter().map(|day| date(*day)).collect(),
        }
    }

    fn steps(count: u32) -> DailyHealth {
        DailyHealth {
            steps: Some(count),
            ..DailyHealth::default()
        }
    }

    #[derive(Default)]
    struct TestProvider {
        open_error: Option<TestError>,
        results: HashMap<NaiveDate, Result<DailyHealth, TestError>>,
        fetched: Mutex<Vec<NaiveDate>>,
    }

    impl HealthProvider for TestProvider {
        type Session = ();
        type Error = TestError;

        async fn open(&self) -> Result<(), TestError> {
            self.open_error.map_or(Ok(()), Err)
        }

        async fn daily(&self, _session: &(), date: NaiveDate) -> Result<DailyHealth, TestError> {
            self.fetched.lock().expect("lock").push(date);
            self.results
                .get(&date)
                .copied()
                .unwrap_or_else(|| Ok(DailyHealth::default()))
        }

        fn is_session_error(error: &TestError) -> bool {
            matches!(error, TestError::Session)
        }

        fn is_throttled(error: &TestError) -> bool {
            matches!(error, TestError::Throttled)
        }
    }

    #[derive(Default)]
    struct TestPublisher {
        failing: HashSet<i64>,
        published: Mutex<Vec<(SourceKind, SourceId, Vec<Measurement>)>>,
    }

    impl Publisher for TestPublisher {
        type Error = io::Error;

        async fn publish(
            &self,
            source_kind: SourceKind,
            source_id: &SourceId,
            measurements: &[Measurement],
        ) -> Result<(), io::Error> {
            if measurements
                .iter()
                .any(|measurement| self.failing.contains(&measurement.timestamp.as_micros()))
            {
                return Err(io::Error::other("broker down"));
            }
            self.published.lock().expect("lock").push((
                source_kind,
                source_id.clone(),
                measurements.to_vec(),
            ));
            Ok(())
        }
    }

    async fn run(
        provider: &TestProvider,
        publisher: &TestPublisher,
        days: &[HealthDay],
    ) -> HealthPull {
        let source_id = SourceId::from("garmin".to_owned());
        collect_health(provider, publisher, &source_id, days).await
    }

    fn stamps(publisher: &TestPublisher) -> Vec<i64> {
        publisher
            .published
            .lock()
            .expect("lock")
            .iter()
            .map(|(_, _, batch)| batch[0].timestamp.as_micros())
            .collect()
    }

    #[tokio::test]
    async fn publishes_each_day_with_its_stamp() {
        let provider = TestProvider {
            results: HashMap::from([(date(1), Ok(steps(5))), (date(2), Ok(steps(7)))]),
            ..TestProvider::default()
        };
        let publisher = TestPublisher::default();
        let outcome = run(&provider, &publisher, &[day(1), day(2)]).await;
        assert_eq!(outcome, completed(&[1, 2]));
        assert_eq!(stamps(&publisher), [1, 2]);
        let published = publisher.published.lock().expect("lock");
        assert_eq!(published[0].0, SourceKind::Health);
        assert_eq!(published[0].1.as_str(), "garmin");
    }

    #[tokio::test]
    async fn an_open_failure_is_classified_and_fetches_and_publishes_nothing() {
        for (error, expected) in [
            (TestError::Throttled, HealthPull::Throttled),
            (TestError::Session, HealthPull::SessionFailed),
            (TestError::Network, HealthPull::Failed),
        ] {
            let provider = TestProvider {
                open_error: Some(error),
                ..TestProvider::default()
            };
            let publisher = TestPublisher::default();
            let outcome = run(&provider, &publisher, &[day(1)]).await;
            assert_eq!(outcome, expected);
            assert!(provider.fetched.lock().expect("lock").is_empty());
            assert!(stamps(&publisher).is_empty());
        }
    }

    #[tokio::test]
    async fn a_failed_day_is_skipped_and_the_next_is_pulled() {
        let provider = TestProvider {
            results: HashMap::from([(date(1), Err(TestError::Day)), (date(2), Ok(steps(7)))]),
            ..TestProvider::default()
        };
        let publisher = TestPublisher::default();
        let outcome = run(&provider, &publisher, &[day(1), day(2)]).await;
        assert_eq!(outcome, completed(&[2]));
        assert_eq!(stamps(&publisher), [2]);
    }

    #[tokio::test]
    async fn a_session_error_on_a_day_aborts_the_pull() {
        let provider = TestProvider {
            results: HashMap::from([(date(1), Err(TestError::Session)), (date(2), Ok(steps(7)))]),
            ..TestProvider::default()
        };
        let publisher = TestPublisher::default();
        let outcome = run(&provider, &publisher, &[day(1), day(2)]).await;
        assert_eq!(outcome, HealthPull::SessionFailed);
        assert_eq!(*provider.fetched.lock().expect("lock"), [date(1)]);
        assert!(stamps(&publisher).is_empty());
    }

    #[tokio::test]
    async fn a_throttled_day_ends_the_pull_without_fetching_later_days() {
        let provider = TestProvider {
            results: HashMap::from([
                (date(1), Err(TestError::Throttled)),
                (date(2), Ok(steps(7))),
            ]),
            ..TestProvider::default()
        };
        let publisher = TestPublisher::default();
        let outcome = run(&provider, &publisher, &[day(1), day(2)]).await;
        assert_eq!(outcome, HealthPull::Throttled);
        assert_eq!(*provider.fetched.lock().expect("lock"), [date(1)]);
        assert!(stamps(&publisher).is_empty());
    }

    #[tokio::test]
    async fn a_failed_publish_is_skipped_and_the_next_day_publishes() {
        let provider = TestProvider {
            results: HashMap::from([(date(1), Ok(steps(5))), (date(2), Ok(steps(7)))]),
            ..TestProvider::default()
        };
        let publisher = TestPublisher {
            failing: HashSet::from([1]),
            ..TestPublisher::default()
        };
        let outcome = run(&provider, &publisher, &[day(1), day(2)]).await;
        assert_eq!(outcome, completed(&[2]));
        assert_eq!(stamps(&publisher), [2]);
    }

    #[tokio::test]
    async fn a_day_without_figures_publishes_nothing() {
        let provider = TestProvider::default();
        let publisher = TestPublisher::default();
        let outcome = run(&provider, &publisher, &[day(1)]).await;
        assert_eq!(outcome, completed(&[1]));
        assert!(stamps(&publisher).is_empty());
    }
}
