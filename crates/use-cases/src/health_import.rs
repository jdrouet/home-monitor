use chrono::NaiveDate;
use domain::Timestamp;

use crate::health_backfill::RunEnd;
use crate::{
    ActivityCursorStore, ActivityImport, ArchiveDays, ArchiveSource, BackfillCursor, BackfillStore,
    HealthBackfill, HealthDay, HealthPull, Publisher, RawArchive,
};

/// What runs after a completed pull, in the same window: the gap fill of the
/// days the daily pulls missed, the yearly backfill, the activities, then the
/// days of Garmin's original documents.
pub struct HealthImport<'a, H, P, A, S, C> {
    pub gap: HealthBackfill<'a, H, P, S>,
    pub yearly: HealthBackfill<'a, H, P, S>,
    pub activities: ActivityImport<'a, H, A, C>,
    pub archive_days: ArchiveDays<'a, H, A, S>,
}

impl<H: ArchiveSource, P: Publisher, A: RawArchive, S: BackfillStore, C: ActivityCursorStore>
    HealthImport<'_, H, P, A, S, C>
{
    /// Does nothing unless `pull` completed. A run that stopped on an error
    /// ends the chain: the runs after it would stop on the same error.
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
        let end = self
            .yearly
            .run(
                |stored| stored.unwrap_or_else(|| BackfillCursor::anchored_at(today)),
                until,
                &clock,
                &day_stamp,
            )
            .await;
        if end == RunEnd::Stopped {
            return;
        }
        if self.activities.run(until, &clock).await == RunEnd::Stopped {
            return;
        }
        self.archive_days
            .run(
                |stored| BackfillCursor::rolling(stored, today, fetched),
                until,
                &clock,
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use chrono::{Days, NaiveDate};
    use domain::{Measurement, SourceId, SourceKind, Timestamp};

    use crate::test_archive::{
        MemoryArchive, MemoryBackfill, MemoryCursor, START, ScriptedSource, clock,
    };
    use crate::{
        ActivityImport, ArchiveDays, HealthBackfill, HealthDay, HealthImport, HealthPull, Publisher,
    };

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
    struct Stores {
        gap: MemoryBackfill,
        yearly: MemoryBackfill,
        archive: MemoryBackfill,
        activities: MemoryCursor,
    }

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
        source: &ScriptedSource,
        archive: &MemoryArchive,
        stores: &Stores,
        pull: HealthPull,
    ) {
        let source_id = SourceId::from("garmin".to_owned());
        let import = HealthImport {
            gap: HealthBackfill {
                provider: source,
                publisher: &TestPublisher,
                store: &stores.gap,
                source_id: &source_id,
            },
            yearly: HealthBackfill {
                provider: source,
                publisher: &TestPublisher,
                store: &stores.yearly,
                source_id: &source_id,
            },
            activities: ActivityImport {
                source,
                archive,
                store: &stores.activities,
            },
            archive_days: ArchiveDays {
                source,
                archive,
                store: &stores.archive,
            },
        };
        let requested = [HealthDay {
            date: yesterday(),
            stamp: stamp(yesterday()),
        }];
        let until = Timestamp::from_micros(START + 86_400_000_000);
        import
            .after_pull(&pull, &requested, today(), until, clock(), stamp)
            .await;
    }

    fn completed() -> HealthPull {
        HealthPull::Completed {
            fetched: vec![yesterday()],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_completed_pull_runs_the_gap_fill_the_yearly_backfill_then_the_archive() {
        let (source, archive, stores) = (
            ScriptedSource::default(),
            MemoryArchive::default(),
            Stores::default(),
        );
        after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.gap.stored().expect("gap cursor").is_done());
        assert!(stores.yearly.stored().expect("yearly cursor").is_done());
        let rolled = stores.archive.stored().expect("archive cursor");
        assert!(rolled.is_done());
        assert_eq!(rolled.last, yesterday());
        assert!(stores.activities.stored().expect("activity cursor").done);
        assert_eq!(archive.count(), 365 * 2);
    }

    #[tokio::test(start_paused = true)]
    async fn other_pulls_import_nothing() {
        for pull in [
            HealthPull::SessionFailed,
            HealthPull::Throttled,
            HealthPull::Failed,
        ] {
            let (source, archive, stores) = (
                ScriptedSource::default(),
                MemoryArchive::default(),
                Stores::default(),
            );
            after_pull(&source, &archive, &stores, pull).await;
            assert_eq!(source.opens(), 0);
            assert_eq!(stores.gap.stored(), None);
            assert_eq!(stores.yearly.stored(), None);
            assert_eq!(stores.archive.stored(), None);
            assert_eq!(stores.activities.stored(), None);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_gap_fill_skips_the_yearly_backfill_and_the_archive() {
        let source = ScriptedSource {
            open_fails: true,
            ..ScriptedSource::default()
        };
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.gap.stored().is_some());
        assert_eq!(stores.yearly.stored(), None);
        assert_eq!(stores.archive.stored(), None);
        assert_eq!(stores.activities.stored(), None);
        assert_eq!(source.opens(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_activity_import_skips_the_archive_days() {
        let source = ScriptedSource {
            list_fails: true,
            ..ScriptedSource::default()
        };
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.yearly.stored().expect("yearly cursor").is_done());
        assert!(!stores.activities.stored().expect("activity cursor").done);
        assert_eq!(stores.archive.stored(), None);
        assert!(source.requested().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_archive_leaves_the_metrics_runs_complete() {
        let source = ScriptedSource::default();
        let archive = MemoryArchive {
            put_fails: true,
            ..MemoryArchive::default()
        };
        let stores = Stores::default();
        after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.gap.stored().expect("gap cursor").is_done());
        assert!(stores.yearly.stored().expect("yearly cursor").is_done());
        let rolled = stores.archive.stored().expect("archive cursor");
        assert!(!rolled.is_done());
        assert_eq!(archive.count(), 0);
    }
}
