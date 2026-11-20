use chrono::NaiveDate;
use domain::{ImportAttempt, ImportJob, Timestamp};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::health_backfill::{RunEnd, RunOutcome};
use crate::import_retry::next_retry;
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
    /// Does nothing unless `pull` completed. A run that Garmin did not answer
    /// is retried alone after 5, 15 and 60 minutes; any other stop ends the
    /// chain. Each run reports its start, its end and its planned retry.
    pub async fn after_pull(
        &self,
        pull: &HealthPull,
        requested: &[HealthDay],
        window: PullWindow<impl Fn() -> Timestamp, impl Fn(NaiveDate) -> Timestamp>,
        record: impl Fn(ImportJob, ImportAttempt, Option<Timestamp>),
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
        let end = tracked(ImportJob::MissedDays, until, &clock, &record, async || {
            self.gap
                .run(
                    |stored| BackfillCursor::after_pull(stored, today, requested, fetched),
                    until,
                    &clock,
                    &day_stamp,
                )
                .await
        })
        .await;
        if matches!(end, RunEnd::Stopped(_)) {
            return;
        }
        let end = tracked(
            ImportJob::HealthHistory,
            until,
            &clock,
            &record,
            async || {
                self.yearly
                    .run(
                        |stored| stored.unwrap_or_else(|| BackfillCursor::anchored_at(today)),
                        until,
                        &clock,
                        &day_stamp,
                    )
                    .await
            },
        )
        .await;
        if matches!(end, RunEnd::Stopped(_)) {
            return;
        }
        let end = tracked(ImportJob::Workouts, until, &clock, &record, async || {
            self.activities.run(until, &clock).await
        })
        .await;
        if matches!(end, RunEnd::Stopped(_)) {
            return;
        }
        tracked(
            ImportJob::DailyArchive,
            until,
            &clock,
            &record,
            async || {
                self.archive_days
                    .run(
                        |stored| BackfillCursor::rolling(stored, today, fetched),
                        until,
                        &clock,
                    )
                    .await
            },
        )
        .await;
    }
}

/// Records `job` as running, awaits `run`, then records how it ended with the
/// retry planned for it. A planned retry runs again after its wait.
async fn tracked(
    job: ImportJob,
    until: Timestamp,
    clock: &impl Fn() -> Timestamp,
    record: &impl Fn(ImportJob, ImportAttempt, Option<Timestamp>),
    run: impl AsyncFn() -> RunOutcome,
) -> RunEnd {
    let mut retries = 0;
    loop {
        let started = clock();
        record(
            job,
            ImportAttempt {
                started,
                ended: None,
            },
            None,
        );
        let outcome = run().await;
        let ended = clock();
        let retry = match outcome.end {
            RunEnd::Stopped(reason) => next_retry(reason, retries, ended, until),
            _ => None,
        };
        record(job, outcome.attempt(started, ended), retry);
        let Some(at) = retry else {
            if retries > 0 && matches!(outcome.end, RunEnd::Stopped(_)) {
                warn!(?job, retries, "garmin import gave up until the next pull");
            }
            return outcome.end;
        };
        info!(
            ?job,
            retry = retries + 1,
            ?at,
            "garmin import failed, retrying"
        );
        sleep(at.duration_since(clock())).await;
        retries += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io;
    use std::sync::Mutex;
    use std::time::Duration;

    use chrono::{Datelike, Days, NaiveDate};
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure, ImportJob,
        Measurement, SourceId, SourceKind, Timestamp,
    };
    use tokio::time::Instant;

    use crate::import_retry::RETRY_DELAYS;
    use crate::import_window::STEP_SPACING;
    use crate::test_archive::{
        MemoryArchive, MemoryBackfill, MemoryCursor, START, ScriptedSource, TestError, after, clock,
    };
    use crate::{
        ActivityCursor, ActivityId, ActivityImport, ActivityRef, ArchiveDays, BackfillCursor,
        DailyHealth, Fetched, HealthBackfill, HealthDay, HealthImport, HealthPull, Publisher,
        PullWindow,
    };

    const DAY: Duration = Duration::from_secs(86_400);

    #[derive(Default)]
    struct TestPublisher {
        stamps: Mutex<Vec<Timestamp>>,
    }

    impl TestPublisher {
        fn stamps(&self) -> Vec<Timestamp> {
            self.stamps.lock().expect("lock").clone()
        }
    }

    impl Publisher for TestPublisher {
        type Error = io::Error;

        async fn publish(
            &self,
            _source_kind: SourceKind,
            _source_id: &SourceId,
            measurements: &[Measurement],
        ) -> Result<(), io::Error> {
            self.stamps
                .lock()
                .expect("lock")
                .push(measurements[0].timestamp);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Stores {
        gap: MemoryBackfill,
        yearly: MemoryBackfill,
        archive: MemoryBackfill,
        activities: MemoryCursor,
        publisher: TestPublisher,
    }

    type Records = Vec<(ImportJob, ImportAttempt, Option<Timestamp>)>;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 15).expect("valid date")
    }

    fn yesterday() -> NaiveDate {
        today() - Days::new(1)
    }

    fn stamp(_date: NaiveDate) -> Timestamp {
        Timestamp::from_micros(0)
    }

    fn dated(date: NaiveDate) -> Timestamp {
        Timestamp::from_micros(i64::from(date.num_days_from_ce()))
    }

    fn plus(at: Timestamp, span: Duration) -> Timestamp {
        Timestamp::from_micros(at.as_micros() + i64::try_from(span.as_micros()).expect("fits"))
    }

    async fn run_until(
        source: &ScriptedSource,
        archive: &MemoryArchive,
        stores: &Stores,
        pull: &HealthPull,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
        day_stamp: impl Fn(NaiveDate) -> Timestamp,
    ) -> Records {
        let source_id = SourceId::from("garmin".to_owned());
        let import = HealthImport {
            gap: HealthBackfill {
                provider: source,
                publisher: &stores.publisher,
                store: &stores.gap,
                source_id: &source_id,
            },
            yearly: HealthBackfill {
                provider: source,
                publisher: &stores.publisher,
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
            until,
            clock,
            day_stamp,
        };
        let records = Mutex::new(Vec::new());
        import
            .after_pull(pull, &requested, window, |job, attempt, retry| {
                records.lock().expect("lock").push((job, attempt, retry));
            })
            .await;
        records.into_inner().expect("lock")
    }

    async fn after_pull(
        source: &ScriptedSource,
        archive: &MemoryArchive,
        stores: &Stores,
        pull: HealthPull,
    ) -> Records {
        run_until(source, archive, stores, &pull, after(DAY), clock(), stamp).await
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
            .map(|(job, attempt, _)| (*job, attempt.ended.is_some()))
            .collect()
    }

    fn end_of(records: &Records, job: ImportJob) -> (AttemptOutcome, ImportCounts) {
        let end = ends(records, job)[0].0;
        (end.outcome, end.counts)
    }

    /// The ended records of `job` with the retry planned at each end.
    fn ends(records: &Records, job: ImportJob) -> Vec<(AttemptEnd, Option<Timestamp>)> {
        records
            .iter()
            .filter(|(recorded, _, _)| *recorded == job)
            .filter_map(|(_, attempt, retry)| attempt.ended.map(|end| (end, *retry)))
            .collect()
    }

    fn starts(records: &Records, job: ImportJob) -> Vec<Timestamp> {
        records
            .iter()
            .filter(|(recorded, attempt, _)| *recorded == job && attempt.ended.is_none())
            .map(|(_, attempt, _)| attempt.started)
            .collect()
    }

    fn failed(reason: ImportFailure) -> AttemptOutcome {
        AttemptOutcome::Failed(reason)
    }

    fn done_gap() -> MemoryBackfill {
        MemoryBackfill::holding(BackfillCursor {
            next: today(),
            last: yesterday(),
        })
    }

    fn done_yearly() -> MemoryBackfill {
        MemoryBackfill::holding(BackfillCursor {
            next: today(),
            last: today() - Days::new(2),
        })
    }

    fn done_activities() -> MemoryCursor {
        MemoryCursor::holding(ActivityCursor {
            start: 0,
            done: true,
            catch_up: None,
            boundary: None,
            pending: None,
        })
    }

    fn activities(count: u64) -> Mutex<Vec<ActivityRef>> {
        Mutex::new(
            (1..=count)
                .map(|id| ActivityRef {
                    id: ActivityId(id),
                    date: yesterday(),
                })
                .collect(),
        )
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
        let source = ScriptedSource::default();
        source.script_opens([Err(TestError::Throttled)]);
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
            failed(ImportFailure::Throttled)
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
        assert_eq!(ends(&records, ImportJob::MissedDays).len(), 1);
        assert_eq!(
            end_of(&records, ImportJob::MissedDays).0,
            failed(ImportFailure::ProgressUnreadable)
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
                .all(|(job, _, _)| *job != ImportJob::DailyArchive)
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
        assert_eq!(ends(&records, ImportJob::DailyArchive).len(), 1);
        assert_eq!(
            end_of(&records, ImportJob::DailyArchive).0,
            failed(ImportFailure::ArchiveUnavailable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_job_retries_after_5_15_and_60_minutes_then_gives_up() {
        let source = ScriptedSource {
            list_fails: true,
            ..ScriptedSource::default()
        };
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        let records = after_pull(&source, &archive, &stores, completed()).await;
        let (ends, starts) = (
            ends(&records, ImportJob::Workouts),
            starts(&records, ImportJob::Workouts),
        );
        assert_eq!((ends.len(), starts.len()), (4, 4));
        for (k, delay) in RETRY_DELAYS.iter().enumerate() {
            let retry = plus(ends[k].0.at, *delay);
            assert_eq!(ends[k].1, Some(retry));
            assert_eq!(starts[k + 1], retry);
        }
        assert_eq!(ends[3].1, None);
        for (end, _) in &ends {
            assert_eq!(end.outcome, failed(ImportFailure::BadAnswer));
        }
        assert!(
            records
                .iter()
                .all(|(job, _, _)| *job != ImportJob::DailyArchive)
        );
    }

    /// Each retry opens a session after the runner's own step spacing, and
    /// the shared request pacer of the Garmin adapter applies as for any run.
    #[tokio::test(start_paused = true)]
    async fn a_retry_that_works_continues_the_chain_after_paced_session_opens() {
        let source = ScriptedSource::default();
        source.script_opens([Err(TestError::Other), Err(TestError::Other)]);
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        let records = after_pull(&source, &archive, &stores, completed()).await;
        let ends_of_gap = ends(&records, ImportJob::MissedDays);
        assert_eq!(ends_of_gap.len(), 3);
        for (end, retry) in &ends_of_gap[..2] {
            assert_eq!(end.outcome, failed(ImportFailure::BadAnswer));
            assert!(retry.is_some());
        }
        assert_eq!(ends_of_gap[2].0.outcome, AttemptOutcome::Done);
        assert_eq!(ends_of_gap[2].1, None);
        for job in &ImportJob::ALL[2..] {
            assert_eq!(end_of(&records, *job).0, AttemptOutcome::Done);
        }
        let opened = source.opened_at();
        assert_eq!(opened[1] - opened[0], RETRY_DELAYS[0] + STEP_SPACING);
        assert_eq!(opened[2] - opened[1], RETRY_DELAYS[1] + STEP_SPACING);
    }

    #[tokio::test(start_paused = true)]
    async fn a_non_retryable_stop_is_a_single_attempt() {
        for (outcome, reason) in [
            (TestError::Session, ImportFailure::LoginRejected),
            (TestError::Throttled, ImportFailure::Throttled),
        ] {
            let source = ScriptedSource::default();
            source.script_opens([Err(outcome)]);
            let (archive, stores) = (MemoryArchive::default(), Stores::default());
            let records = after_pull(&source, &archive, &stores, completed()).await;
            let ends = ends(&records, ImportJob::MissedDays);
            assert_eq!(ends.len(), 1);
            assert_eq!(ends[0].0.outcome, failed(reason));
            assert_eq!(ends[0].1, None);
            assert_eq!(source.opens(), 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_backoff_crossing_the_deadline_gives_up_early() {
        let source = ScriptedSource::default();
        source.script_opens([Err(TestError::Other), Err(TestError::Other)]);
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        let until = after(Duration::from_secs(25 * 60));
        let clock = clock();
        let records = run_until(
            &source,
            &archive,
            &stores,
            &completed(),
            until,
            &clock,
            stamp,
        )
        .await;
        let ends = ends(&records, ImportJob::MissedDays);
        assert_eq!(ends.len(), 2);
        assert!(ends[0].1.is_some());
        assert_eq!(ends[1].1, None);
        assert!(clock() < until);
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_wake_from_the_backoff_starts_nothing() {
        let source = ScriptedSource::default();
        source.script_opens([Err(TestError::Other)]);
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        let start = Instant::now();
        let late = move || {
            let elapsed = start.elapsed();
            let skew = if elapsed >= RETRY_DELAYS[0] + STEP_SPACING {
                Duration::from_secs(15 * 60)
            } else {
                Duration::ZERO
            };
            plus(Timestamp::from_micros(START), elapsed + skew)
        };
        let records = run_until(
            &source,
            &archive,
            &stores,
            &completed(),
            after(Duration::from_secs(25 * 60)),
            late,
            stamp,
        )
        .await;
        let ends = ends(&records, ImportJob::MissedDays);
        assert_eq!(ends.len(), 2);
        assert!(ends[0].1.is_some());
        assert_eq!(ends[1].0.outcome, AttemptOutcome::Paused);
        assert_eq!(source.opens(), 1);
        assert!(source.days().is_empty());
        assert!(source.requested().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_step_that_was_admitted_finishes_and_no_further_step_starts() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(40 * 60),
            ..ScriptedSource::default()
        };
        let archive = MemoryArchive::default();
        let stores = Stores {
            gap: done_gap(),
            yearly: done_yearly(),
            activities: done_activities(),
            ..Stores::default()
        };
        let until = after(Duration::from_secs(60 * 60));
        let clock = clock();
        let records = run_until(
            &source,
            &archive,
            &stores,
            &completed(),
            until,
            &clock,
            stamp,
        )
        .await;
        let ends = ends(&records, ImportJob::DailyArchive);
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].0.outcome, AttemptOutcome::Paused);
        assert_eq!(ends[0].1, None);
        assert_eq!(source.requested().len(), 2);
        assert!(clock() > until);
    }

    #[tokio::test(start_paused = true)]
    async fn an_archive_retry_fetches_the_failed_document_again_and_stores_each_once() {
        let source = ScriptedSource::default();
        source.script([Ok(Fetched::Body(b"a".to_vec())), Err(TestError::Other)]);
        let (archive, stores) = (MemoryArchive::default(), Stores::default());
        let records = after_pull(&source, &archive, &stores, completed()).await;
        let ends = ends(&records, ImportJob::DailyArchive);
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[0].0.outcome, failed(ImportFailure::BadAnswer));
        assert!(ends[0].1.is_some());
        assert_eq!(ends[1].0.outcome, AttemptOutcome::Done);
        let requested = source.requested();
        let failed_key = format!("{:?}", requested[1]);
        let mut asked: HashMap<String, usize> = HashMap::new();
        for request in &requested {
            *asked.entry(format!("{request:?}")).or_default() += 1;
        }
        for (key, times) in &asked {
            assert_eq!(*times, if *key == failed_key { 2 } else { 1 }, "{key}");
        }
        let writes = archive.writes();
        assert_eq!(archive.count(), 365 * 2);
        assert_eq!(writes.len(), 365 * 2);
        assert!(writes.values().all(|times| *times == 1));
        assert!(stores.archive.stored().expect("archive cursor").is_done());
    }

    #[tokio::test(start_paused = true)]
    async fn a_workout_retry_resumes_after_a_failure_inside_a_page() {
        let source = ScriptedSource {
            list: activities(10),
            ..ScriptedSource::default()
        };
        let ok = || Ok(Fetched::Body(b"x".to_vec()));
        source.script((0..13).map(|_| ok()).chain([Err(TestError::Other)]));
        let archive = MemoryArchive::default();
        let stores = Stores {
            gap: done_gap(),
            yearly: done_yearly(),
            ..Stores::default()
        };
        let records = after_pull(&source, &archive, &stores, completed()).await;
        let ends = ends(&records, ImportJob::Workouts);
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[0].0.outcome, failed(ImportFailure::BadAnswer));
        assert_eq!(ends[1].0.outcome, AttemptOutcome::Done);
        let activity_keys = 10 * 2;
        let writes = archive.writes();
        assert_eq!(writes.len(), activity_keys + 365 * 2);
        assert!(writes.values().all(|times| *times == 1));
        let saved = stores.activities.saves();
        assert!(saved.iter().any(|cursor| cursor.start == 5 && !cursor.done));
        assert!(stores.activities.stored().expect("activity cursor").done);
    }

    #[tokio::test(start_paused = true)]
    async fn the_catch_up_reads_ten_completed_pages_per_attempt_and_is_not_retried() {
        let source = ScriptedSource {
            list: activities(60),
            ..ScriptedSource::default()
        };
        let archive = MemoryArchive::default();
        let stores = Stores {
            gap: done_gap(),
            yearly: done_yearly(),
            activities: done_activities(),
            ..Stores::default()
        };
        let records = after_pull(&source, &archive, &stores, completed()).await;
        let ends = ends(&records, ImportJob::Workouts);
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].0.outcome, AttemptOutcome::Capped);
        assert_eq!(ends[0].1, None);
        let page_reads = source
            .pages()
            .iter()
            .filter(|(_, limit)| *limit == 5)
            .count();
        assert_eq!(page_reads, 10);
        assert_eq!(
            end_of(&records, ImportJob::DailyArchive).0,
            AttemptOutcome::Done
        );
    }

    /// Runs `job` with a failing session reopen and returns the days Garmin
    /// was asked for and the stamps that were published.
    async fn retried_health_job(
        job: ImportJob,
        stores: Stores,
    ) -> (Vec<NaiveDate>, Vec<Timestamp>) {
        let source = ScriptedSource {
            daily: DailyHealth {
                steps: Some(1),
                ..DailyHealth::default()
            },
            ..ScriptedSource::default()
        };
        source.script_opens([Ok(()), Err(TestError::Other)]);
        let archive = MemoryArchive::default();
        let records = run_until(
            &source,
            &archive,
            &stores,
            &completed(),
            after(DAY),
            clock(),
            dated,
        )
        .await;
        let ends = ends(&records, job);
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[0].0.outcome, failed(ImportFailure::BadAnswer));
        assert!(ends[0].1.is_some());
        assert_eq!(ends[1].0.outcome, AttemptOutcome::Done);
        (source.days(), stores.publisher.stamps())
    }

    fn assert_each_once(days: &[NaiveDate], stamps: &[Timestamp], expected: usize) {
        let mut unique = days.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!((days.len(), unique.len()), (expected, expected));
        let mut published = stamps.to_vec();
        published.sort();
        published.dedup();
        assert_eq!((stamps.len(), published.len()), (expected, expected));
        let wanted: Vec<_> = unique.into_iter().map(dated).collect();
        assert_eq!(published, wanted);
    }

    #[tokio::test(start_paused = true)]
    async fn a_gap_fill_retry_publishes_each_day_once() {
        let stores = Stores {
            yearly: done_yearly(),
            ..Stores::default()
        };
        let (days, stamps) = retried_health_job(ImportJob::MissedDays, stores).await;
        assert_each_once(&days, &stamps, 31);
    }

    #[tokio::test(start_paused = true)]
    async fn a_yearly_backfill_retry_publishes_each_day_once() {
        let stores = Stores {
            gap: done_gap(),
            ..Stores::default()
        };
        let (days, stamps) = retried_health_job(ImportJob::HealthHistory, stores).await;
        assert_each_once(&days, &stamps, 364);
    }
}
