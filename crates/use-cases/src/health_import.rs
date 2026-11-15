use chrono::NaiveDate;
use domain::Timestamp;

use crate::health_backfill::RunEnd;
use crate::{
    BackfillCursor, BackfillStore, HealthBackfill, HealthDay, HealthProvider, HealthPull, Publisher,
};

/// What runs after a completed pull: the gap fill of the days the daily pulls
/// missed, then the yearly backfill, in the same window.
pub struct HealthImport<'a, H, P, G, B> {
    pub gap: HealthBackfill<'a, H, P, G>,
    pub yearly: HealthBackfill<'a, H, P, B>,
}

impl<H: HealthProvider, P: Publisher, G: BackfillStore, B: BackfillStore>
    HealthImport<'_, H, P, G, B>
{
    /// Does nothing unless `pull` completed. The yearly backfill is skipped
    /// when the gap fill stopped on an error, which would stop it too.
    pub async fn after_pull(
        &self,
        pull: &HealthPull,
        requested: &[HealthDay],
        today: NaiveDate,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
        day_stamp: impl Fn(NaiveDate) -> Timestamp,
    ) {
        let HealthPull::Completed { fetched } = pull else {
            return;
        };
        let end = self
            .gap
            .run(
                |stored| BackfillCursor::after_pull(stored, today, requested, fetched),
                until,
                &clock,
                &day_stamp,
            )
            .await;
        if end == RunEnd::Stopped {
            return;
        }
        self.yearly
            .run(
                |stored| stored.unwrap_or_else(|| BackfillCursor::anchored_at(today)),
                until,
                &clock,
                &day_stamp,
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Mutex;

    use chrono::{Days, NaiveDate};
    use domain::{Measurement, SourceId, SourceKind, Timestamp};
    use tokio::time::Instant;

    use crate::{
        BackfillCursor, BackfillStore, DailyHealth, HealthBackfill, HealthDay, HealthImport,
        HealthProvider, HealthPull, Publisher,
    };

    #[derive(Debug)]
    struct TestError;

    impl std::fmt::Display for TestError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("test error")
        }
    }

    impl std::error::Error for TestError {}

    #[derive(Default)]
    struct TestProvider {
        open_fails: bool,
        opens: Mutex<usize>,
    }

    impl HealthProvider for TestProvider {
        type Session = ();
        type Error = TestError;

        async fn open(&self) -> Result<(), TestError> {
            *self.opens.lock().expect("lock") += 1;
            if self.open_fails {
                Err(TestError)
            } else {
                Ok(())
            }
        }

        async fn daily(&self, _session: &(), _date: NaiveDate) -> Result<DailyHealth, TestError> {
            Ok(DailyHealth::default())
        }

        fn is_session_error(_error: &TestError) -> bool {
            true
        }

        fn is_throttled(_error: &TestError) -> bool {
            false
        }
    }

    struct TestPublisher;

    impl Publisher for TestPublisher {
        type Error = io::Error;

        async fn publish(
            &self,
            _source_kind: SourceKind,
            _source_id: &SourceId,
            _measurements: &[Measurement],
        ) -> Result<(), io::Error> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestStore(Mutex<Option<BackfillCursor>>);

    impl TestStore {
        fn stored(&self) -> Option<BackfillCursor> {
            *self.0.lock().expect("lock")
        }
    }

    impl BackfillStore for TestStore {
        type Error = io::Error;

        fn load(&self) -> Result<Option<BackfillCursor>, io::Error> {
            Ok(self.stored())
        }

        fn save(&self, cursor: &BackfillCursor) -> Result<(), io::Error> {
            *self.0.lock().expect("lock") = Some(*cursor);
            Ok(())
        }
    }

    const START: i64 = 1_000_000_000_000_000;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 15).expect("valid date")
    }

    fn yesterday() -> NaiveDate {
        today() - Days::new(1)
    }

    fn stamp(_date: NaiveDate) -> Timestamp {
        Timestamp::from_micros(0)
    }

    async fn after_pull(
        provider: &TestProvider,
        gap: &TestStore,
        yearly: &TestStore,
        pull: HealthPull,
    ) {
        let source_id = SourceId::from("garmin".to_owned());
        let import = HealthImport {
            gap: HealthBackfill {
                provider,
                publisher: &TestPublisher,
                store: gap,
                source_id: &source_id,
            },
            yearly: HealthBackfill {
                provider,
                publisher: &TestPublisher,
                store: yearly,
                source_id: &source_id,
            },
        };
        let start = Instant::now();
        let clock = move || {
            let elapsed = i64::try_from(start.elapsed().as_micros()).expect("fits");
            Timestamp::from_micros(START + elapsed)
        };
        let requested = [HealthDay {
            date: yesterday(),
            stamp: stamp(yesterday()),
        }];
        let until = Timestamp::from_micros(START + 86_400_000_000);
        import
            .after_pull(&pull, &requested, today(), until, clock, stamp)
            .await;
    }

    fn completed() -> HealthPull {
        HealthPull::Completed {
            fetched: vec![yesterday()],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_completed_pull_runs_the_gap_fill_then_the_yearly_backfill() {
        let provider = TestProvider::default();
        let (gap, yearly) = (TestStore::default(), TestStore::default());
        after_pull(&provider, &gap, &yearly, completed()).await;
        assert!(gap.stored().expect("gap cursor").is_done());
        assert!(yearly.stored().expect("yearly cursor").is_done());
    }

    #[tokio::test(start_paused = true)]
    async fn other_pulls_import_nothing() {
        for pull in [
            HealthPull::SessionFailed,
            HealthPull::Throttled,
            HealthPull::Failed,
        ] {
            let provider = TestProvider::default();
            let (gap, yearly) = (TestStore::default(), TestStore::default());
            after_pull(&provider, &gap, &yearly, pull).await;
            assert_eq!(*provider.opens.lock().expect("lock"), 0);
            assert_eq!(gap.stored(), None);
            assert_eq!(yearly.stored(), None);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_gap_fill_skips_the_yearly_backfill() {
        let provider = TestProvider {
            open_fails: true,
            ..TestProvider::default()
        };
        let (gap, yearly) = (TestStore::default(), TestStore::default());
        after_pull(&provider, &gap, &yearly, completed()).await;
        assert!(gap.stored().is_some());
        assert_eq!(yearly.stored(), None);
        assert_eq!(*provider.opens.lock().expect("lock"), 1);
    }
}
