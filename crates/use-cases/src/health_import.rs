use std::future::Future;

use chrono::NaiveDate;
use domain::{ImportAttempt, ImportJob, Timestamp};

use crate::health_backfill::{RunEnd, RunOutcome};
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

/// The day and the time bounds of one pull and the imports that follow it.
pub struct PullWindow<C, D> {
    pub today: NaiveDate,
    /// The next scheduled pull, which the imports must not delay.
    pub until: Timestamp,
    pub clock: C,
    pub day_stamp: D,
}

impl<H: ArchiveSource, P: Publisher, A: RawArchive, S: BackfillStore, C: ActivityCursorStore>
    HealthImport<'_, H, P, A, S, C>
{
    /// Does nothing unless `pull` completed. A run that stopped on an error
    /// ends the chain: the runs after it would stop on the same error. Each
    /// run reports its start and its end through `record`.
    pub async fn after_pull(
        &self,
        pull: &HealthPull,
        requested: &[HealthDay],
        window: PullWindow<impl Fn() -> Timestamp, impl Fn(NaiveDate) -> Timestamp>,
        record: impl Fn(ImportJob, ImportAttempt),
    ) {
        let HealthPull::Completed { fetched, .. } = pull else {
            return;
        };
        let PullWindow {
            today,
            until,
            clock,
            day_stamp,
        } = window;
        let end = tracked(
            ImportJob::MissedDays,
            &clock,
            &record,
            self.gap.run(
                |stored| BackfillCursor::after_pull(stored, today, requested, fetched),
                until,
                &clock,
                &day_stamp,
            ),
        )
        .await;
        if matches!(end, RunEnd::Stopped(_)) {
            return;
        }
        let end = tracked(
            ImportJob::HealthHistory,
            &clock,
            &record,
            self.yearly.run(
                |stored| stored.unwrap_or_else(|| BackfillCursor::anchored_at(today)),
                until,
                &clock,
                &day_stamp,
            ),
        )
        .await;
        if matches!(end, RunEnd::Stopped(_)) {
            return;
        }
        let end = tracked(
            ImportJob::Workouts,
            &clock,
            &record,
            self.activities.run(until, &clock),
        )
        .await;
        if matches!(end, RunEnd::Stopped(_)) {
            return;
        }
        tracked(
            ImportJob::DailyArchive,
            &clock,
            &record,
            self.archive_days.run(
                |stored| BackfillCursor::rolling(stored, today, fetched),
                until,
                &clock,
            ),
        )
        .await;
    }
}

/// Records `job` as running, awaits `run`, then records how it ended.
async fn tracked(
    job: ImportJob,
    clock: &impl Fn() -> Timestamp,
    record: &impl Fn(ImportJob, ImportAttempt),
    run: impl Future<Output = RunOutcome>,
) -> RunEnd {
    let started = clock();
    record(
        job,
        ImportAttempt {
            started,
            ended: None,
        },
    );
    let outcome = run.await;
    record(job, outcome.attempt(started, clock()));
    outcome.end
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Mutex;

    use chrono::{Days, NaiveDate};
    use domain::{
        AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure, ImportJob, Measurement,
        SourceId, SourceKind, Timestamp,
    };

    use crate::test_archive::{
        MemoryArchive, MemoryBackfill, MemoryCursor, START, ScriptedSource, clock,
    };
    use crate::{
        ActivityImport, ArchiveDays, HealthBackfill, HealthDay, HealthImport, HealthPull,
        Publisher, PullWindow,
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

    type Records = Vec<(ImportJob, ImportAttempt)>;

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
    ) -> Records {
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
        let window = PullWindow {
            today: today(),
            until: Timestamp::from_micros(START + 86_400_000_000),
            clock: clock(),
            day_stamp: stamp,
        };
        let records = Mutex::new(Vec::new());
        import
            .after_pull(&pull, &requested, window, |job, attempt| {
                records.lock().expect("lock").push((job, attempt));
            })
            .await;
        records.into_inner().expect("lock")
    }

    fn completed() -> HealthPull {
        HealthPull::Completed {
            fetched: vec![yesterday()],
            counts: ImportCounts::default(),
        }
    }

    fn jobs(records: &Records) -> Vec<(ImportJob, bool)> {
        records
            .iter()
            .map(|(job, attempt)| (*job, attempt.ended.is_some()))
            .collect()
    }

    fn end_of(records: &Records, job: ImportJob) -> (AttemptOutcome, ImportCounts) {
        let (_, attempt) = records
            .iter()
            .find(|(recorded, attempt)| *recorded == job && attempt.ended.is_some())
            .expect("an ended record");
        let end = attempt.ended.expect("ended");
        (end.outcome, end.counts)
    }

    #[tokio::test(start_paused = true)]
    async fn a_completed_pull_runs_the_gap_fill_the_yearly_backfill_then_the_archive() {
        let (source, archive, stores) = (
            ScriptedSource::default(),
            MemoryArchive::default(),
            Stores::default(),
        );
        let records = after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.gap.stored().expect("gap cursor").is_done());
        assert!(stores.yearly.stored().expect("yearly cursor").is_done());
        let rolled = stores.archive.stored().expect("archive cursor");
        assert!(rolled.is_done());
        assert_eq!(rolled.last, yesterday());
        assert!(stores.activities.stored().expect("activity cursor").done);
        assert_eq!(archive.count(), 365 * 2);
        assert_eq!(
            jobs(&records),
            [
                (ImportJob::MissedDays, false),
                (ImportJob::MissedDays, true),
                (ImportJob::HealthHistory, false),
                (ImportJob::HealthHistory, true),
                (ImportJob::Workouts, false),
                (ImportJob::Workouts, true),
                (ImportJob::DailyArchive, false),
                (ImportJob::DailyArchive, true),
            ]
        );
        for job in &ImportJob::ALL[1..] {
            assert_eq!(end_of(&records, *job).0, AttemptOutcome::Done);
        }
        assert_eq!(end_of(&records, ImportJob::HealthHistory).1.empty, 364);
        assert_eq!(end_of(&records, ImportJob::DailyArchive).1.stored, 365);
    }

    #[tokio::test(start_paused = true)]
    async fn other_pulls_import_and_record_nothing() {
        for pull in [
            HealthPull::SessionFailed {
                counts: ImportCounts::default(),
            },
            HealthPull::Throttled {
                counts: ImportCounts::default(),
            },
            HealthPull::Failed(ImportFailure::Unreachable),
        ] {
            let (source, archive, stores) = (
                ScriptedSource::default(),
                MemoryArchive::default(),
                Stores::default(),
            );
            let records = after_pull(&source, &archive, &stores, pull).await;
            assert!(records.is_empty());
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
        let records = after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.gap.stored().is_some());
        assert_eq!(stores.yearly.stored(), None);
        assert_eq!(stores.archive.stored(), None);
        assert_eq!(stores.activities.stored(), None);
        assert_eq!(source.opens(), 1);
        assert_eq!(
            jobs(&records),
            [
                (ImportJob::MissedDays, false),
                (ImportJob::MissedDays, true)
            ]
        );
        assert_eq!(
            end_of(&records, ImportJob::MissedDays).0,
            AttemptOutcome::Failed(ImportFailure::BadAnswer)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_gap_cursor_fails_that_job_only_and_the_chain_goes_on() {
        let (source, archive) = (ScriptedSource::default(), MemoryArchive::default());
        let stores = Stores {
            gap: MemoryBackfill {
                load_fails: true,
                ..MemoryBackfill::default()
            },
            ..Stores::default()
        };
        let records = after_pull(&source, &archive, &stores, completed()).await;
        assert_eq!(
            end_of(&records, ImportJob::MissedDays).0,
            AttemptOutcome::Failed(ImportFailure::ProgressUnreadable)
        );
        assert_eq!(
            end_of(&records, ImportJob::DailyArchive).0,
            AttemptOutcome::Done
        );
        assert!(stores.yearly.stored().expect("yearly cursor").is_done());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_activity_import_skips_the_archive_days() {
        let source = ScriptedSource {
            list_fails: true,
            ..ScriptedSource::default()
        };
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        let records = after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.yearly.stored().expect("yearly cursor").is_done());
        assert!(!stores.activities.stored().expect("activity cursor").done);
        assert_eq!(stores.archive.stored(), None);
        assert!(source.requested().is_empty());
        assert_eq!(
            end_of(&records, ImportJob::Workouts).0,
            AttemptOutcome::Failed(ImportFailure::BadAnswer)
        );
        assert!(
            records
                .iter()
                .all(|(job, _)| *job != ImportJob::DailyArchive)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_archive_leaves_the_metrics_runs_complete() {
        let source = ScriptedSource::default();
        let archive = MemoryArchive {
            put_fails: true,
            ..MemoryArchive::default()
        };
        let stores = Stores::default();
        let records = after_pull(&source, &archive, &stores, completed()).await;
        assert!(stores.gap.stored().expect("gap cursor").is_done());
        assert!(stores.yearly.stored().expect("yearly cursor").is_done());
        let rolled = stores.archive.stored().expect("archive cursor");
        assert!(!rolled.is_done());
        assert_eq!(archive.count(), 0);
        assert_eq!(
            end_of(&records, ImportJob::DailyArchive).0,
            AttemptOutcome::Failed(ImportFailure::ArchiveUnavailable)
        );
    }
}
