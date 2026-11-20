use chrono::NaiveDate;
use domain::{
    AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure, SourceId, SourceKind,
    Timestamp,
};
use tracing::{error, warn};

use crate::{HealthDay, HealthProvider, Publisher};

/// How a pull ended. The alerter raises the login alert on `SessionFailed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthPull {
    /// The pull ran to the end. `fetched` lists the days whose fetch worked
    /// and whose publish worked or had nothing to publish.
    Completed {
        fetched: Vec<NaiveDate>,
        counts: ImportCounts,
    },
    /// `counts` holds the days that passed before the failure.
    SessionFailed { counts: ImportCounts },
    /// Garmin asked the client to slow down.
    Throttled { counts: ImportCounts },
    /// The session could not open for a reason a login would not fix, such
    /// as a network error or an outage. No day was tried.
    Failed(ImportFailure),
}

impl HealthPull {
    /// The finished attempt that started at `started`.
    pub fn attempt(&self, started: Timestamp, ended: Timestamp) -> ImportAttempt {
        let (outcome, counts) = match self {
            Self::Completed { counts, .. } => (AttemptOutcome::Done, *counts),
            Self::SessionFailed { counts } => (
                AttemptOutcome::Failed(ImportFailure::LoginRejected),
                *counts,
            ),
            Self::Throttled { counts } => {
                (AttemptOutcome::Failed(ImportFailure::Throttled), *counts)
            }
            Self::Failed(reason) => (AttemptOutcome::Failed(*reason), ImportCounts::default()),
        };
        ImportAttempt {
            started,
            ended: Some(AttemptEnd {
                at: ended,
                outcome,
                counts,
            }),
        }
    }
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
            return HealthPull::Throttled {
                counts: ImportCounts::default(),
            };
        }
        Err(error) if H::is_session_error(&error) => {
            error!(%error, "health session failed, skipping this pull");
            return HealthPull::SessionFailed {
                counts: ImportCounts::default(),
            };
        }
        Err(error) => {
            error!(%error, "health session could not open, skipping this pull");
            return HealthPull::Failed(H::failure(&error));
        }
    };
    let mut fetched = Vec::new();
    let mut counts = ImportCounts::default();
    for day in days {
        let health = match provider.daily(&session, day.date).await {
            Ok(health) => health,
            Err(error) if H::is_session_error(&error) => {
                error!(%error, "health session rejected, skipping this pull");
                return HealthPull::SessionFailed { counts };
            }
            Err(error) if H::is_throttled(&error) => {
                warn!(%error, "health fetch throttled, ending this pull");
                return HealthPull::Throttled { counts };
            }
            Err(error) => {
                warn!(%error, date = %day.date, "health fetch failed, skipping this day");
                counts.failed += 1;
                continue;
            }
        };
        let batch = health.measurements(source_id, day.stamp);
        if batch.is_empty() {
            fetched.push(day.date);
            counts.empty += 1;
            continue;
        }
        match publisher
            .publish(SourceKind::Health, source_id, &batch)
            .await
        {
            Ok(()) => {
                fetched.push(day.date);
                counts.stored += 1;
            }
            Err(error) => {
                warn!(%error, date = %day.date, "health publish failed, skipping this day");
                counts.failed += 1;
            }
        }
    }
    HealthPull::Completed { fetched, counts }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fmt;
    use std::io;
    use std::sync::Mutex;

    use chrono::NaiveDate;
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure, Measurement,
        SourceId, SourceKind, Timestamp,
    };

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

    fn counts(stored: u32, empty: u32, failed: u32) -> ImportCounts {
        ImportCounts {
            stored,
            empty,
            failed,
        }
    }

    fn completed(days: &[u32], counts: ImportCounts) -> HealthPull {
        HealthPull::Completed {
            fetched: days.iter().map(|day| date(*day)).collect(),
            counts,
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
        assert_eq!(outcome, completed(&[1, 2], counts(2, 0, 0)));
        assert_eq!(stamps(&publisher), [1, 2]);
        let published = publisher.published.lock().expect("lock");
        assert_eq!(published[0].0, SourceKind::Health);
        assert_eq!(published[0].1.as_str(), "garmin");
    }

    #[tokio::test]
    async fn an_open_failure_is_classified_and_fetches_and_publishes_nothing() {
        for (error, expected) in [
            (
                TestError::Throttled,
                HealthPull::Throttled {
                    counts: ImportCounts::default(),
                },
            ),
            (
                TestError::Session,
                HealthPull::SessionFailed {
                    counts: ImportCounts::default(),
                },
            ),
            (
                TestError::Network,
                HealthPull::Failed(ImportFailure::BadAnswer),
            ),
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
        assert_eq!(outcome, completed(&[2], counts(1, 0, 1)));
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
        assert_eq!(
            outcome,
            HealthPull::SessionFailed {
                counts: ImportCounts::default()
            }
        );
        assert_eq!(*provider.fetched.lock().expect("lock"), [date(1)]);
        assert!(stamps(&publisher).is_empty());
    }

    #[tokio::test]
    async fn a_stop_after_a_stored_day_keeps_its_count() {
        for (error, expected) in [
            (
                TestError::Session,
                HealthPull::SessionFailed {
                    counts: counts(1, 0, 0),
                },
            ),
            (
                TestError::Throttled,
                HealthPull::Throttled {
                    counts: counts(1, 0, 0),
                },
            ),
        ] {
            let provider = TestProvider {
                results: HashMap::from([(date(1), Ok(steps(5))), (date(2), Err(error))]),
                ..TestProvider::default()
            };
            let publisher = TestPublisher::default();
            let outcome = run(&provider, &publisher, &[day(1), day(2)]).await;
            assert_eq!(outcome, expected);
            assert_eq!(stamps(&publisher), [1]);
        }
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
        assert_eq!(
            outcome,
            HealthPull::Throttled {
                counts: ImportCounts::default()
            }
        );
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
        assert_eq!(outcome, completed(&[2], counts(1, 0, 1)));
        assert_eq!(stamps(&publisher), [2]);
    }

    #[tokio::test]
    async fn a_day_without_figures_publishes_nothing() {
        let provider = TestProvider::default();
        let publisher = TestPublisher::default();
        let outcome = run(&provider, &publisher, &[day(1)]).await;
        assert_eq!(outcome, completed(&[1], counts(0, 1, 0)));
        assert!(stamps(&publisher).is_empty());
    }

    #[test]
    fn an_attempt_carries_the_outcome_and_the_counts() {
        let (started, ended) = (Timestamp::from_micros(1), Timestamp::from_micros(2));
        let table = [
            (
                completed(&[1], counts(1, 2, 3)),
                AttemptOutcome::Done,
                counts(1, 2, 3),
            ),
            (
                HealthPull::SessionFailed {
                    counts: counts(1, 0, 0),
                },
                AttemptOutcome::Failed(ImportFailure::LoginRejected),
                counts(1, 0, 0),
            ),
            (
                HealthPull::Throttled {
                    counts: counts(0, 1, 0),
                },
                AttemptOutcome::Failed(ImportFailure::Throttled),
                counts(0, 1, 0),
            ),
            (
                HealthPull::Failed(ImportFailure::GarminDown),
                AttemptOutcome::Failed(ImportFailure::GarminDown),
                ImportCounts::default(),
            ),
        ];
        for (pull, outcome, counts) in table {
            assert_eq!(
                pull.attempt(started, ended),
                ImportAttempt {
                    started,
                    ended: Some(AttemptEnd {
                        at: ended,
                        outcome,
                        counts,
                    }),
                }
            );
        }
    }
}
