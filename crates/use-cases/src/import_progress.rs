use std::time::Duration;

use chrono::Days;
use domain::{
    AttemptOutcome, ImportAttempt, ImportEta, ImportFailure, ImportJob, ImportProgress,
    ImportStatus, Timestamp,
};

use crate::import_window::{MARGIN, REQUEST_GAP, STEP_SPACING};
use crate::{ActivityCursor, ActivityCursorStore, ArchiveSource, BackfillCursor, BackfillStore};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// What one pull's window offers the imports: a day minus the safety margin.
const WINDOW: Duration = DAY.saturating_sub(MARGIN);
/// One 60 s step plus its share of the 60 s session reopen every 30 min.
const HEALTH_DAY_COST: Duration = Duration::from_secs(62);
/// The jobs the chain runs after the daily pull, in order.
const CHAIN: [ImportJob; 4] = [
    ImportJob::MissedDays,
    ImportJob::HealthHistory,
    ImportJob::Workouts,
    ImportJob::DailyArchive,
];

/// The cursors of the four chained jobs.
pub struct ImportCursors<'a, S, C> {
    pub gap: &'a S,
    pub yearly: &'a S,
    pub activities: &'a C,
    pub archive: &'a S,
}

/// What a job still has to do, as far as its cursor tells.
enum Work {
    Nothing,
    Days(u32),
    Unknown,
}

struct Loaded {
    gap: Option<BackfillCursor>,
    yearly: Option<BackfillCursor>,
    archive: Option<BackfillCursor>,
    activities: Option<ActivityCursor>,
}

impl<S: BackfillStore, C: ActivityCursorStore> ImportCursors<'_, S, C> {
    /// `jobs` with each job's progress from its cursor and a finish estimate
    /// built from the per-job costs. An unreadable or missing cursor gives no
    /// progress and an unknown finish. Without `jobs.next_pull` there is no
    /// estimate.
    pub fn status<H: ArchiveSource>(&self, mut jobs: ImportStatus, now: Timestamp) -> ImportStatus {
        let loaded = Loaded {
            gap: self.gap.load().ok().flatten(),
            yearly: self.yearly.load().ok().flatten(),
            archive: self.archive.load().ok().flatten(),
            activities: self.activities.load().ok().flatten(),
        };
        let documents = u32::try_from(H::DAILY.len()).unwrap_or(u32::MAX);
        let archive_day = STEP_SPACING + REQUEST_GAP * documents;
        for job in CHAIN {
            let day = if job == ImportJob::DailyArchive {
                archive_day
            } else {
                HEALTH_DAY_COST
            };
            let eta = jobs
                .next_pull
                .and_then(|next| estimate(&jobs, &loaded, job, day, now, next));
            let entry = jobs.jobs.entry(job).or_default();
            entry.progress = loaded.progress(job);
            entry.eta = eta;
        }
        jobs
    }
}

impl Loaded {
    fn days(&self, job: ImportJob) -> Option<BackfillCursor> {
        match job {
            ImportJob::MissedDays => self.gap,
            ImportJob::HealthHistory => self.yearly,
            _ => self.archive,
        }
    }

    fn progress(&self, job: ImportJob) -> Option<ImportProgress> {
        if job == ImportJob::Workouts {
            return self.activities.map(|cursor| ImportProgress::Workouts {
                complete: cursor.done,
            });
        }
        let cursor = self.days(job)?;
        Some(ImportProgress::Days {
            done_through: cursor.next.checked_sub_days(Days::new(1))?,
            left: left(cursor),
        })
    }

    fn work(&self, job: ImportJob) -> Work {
        if job == ImportJob::Workouts {
            return match self.activities {
                Some(cursor) if cursor.done && cursor.catch_up.is_none() => Work::Nothing,
                _ => Work::Unknown,
            };
        }
        match self.days(job) {
            None => Work::Unknown,
            Some(cursor) => match left(cursor) {
                0 => Work::Nothing,
                days => Work::Days(days),
            },
        }
    }
}

fn left(cursor: BackfillCursor) -> u32 {
    if cursor.is_done() {
        return 0;
    }
    u32::try_from((cursor.last - cursor.next).num_days() + 1).unwrap_or(u32::MAX)
}

/// The job running now, or the one that resumes first after a pause, gets a
/// time or a count of further windows; a job that an earlier one provably
/// holds up waits for it; anything else is unknown.
fn estimate(
    jobs: &ImportStatus,
    loaded: &Loaded,
    job: ImportJob,
    day: Duration,
    now: Timestamp,
    next: Timestamp,
) -> Option<ImportEta> {
    let work = loaded.work(job);
    if matches!(work, Work::Nothing) {
        return None;
    }
    let ended = latest(jobs, job).map(|attempt| attempt.ended);
    let running = ended == Some(None);
    let paused = ended
        .flatten()
        .is_some_and(|end| end.outcome == AttemptOutcome::Paused);
    if !running {
        if let Some(blocking) = blocker(jobs, loaded, job, next) {
            return Some(ImportEta::WaitsFor { job: blocking });
        }
        if !paused {
            return Some(ImportEta::Unknown);
        }
    }
    let Work::Days(left) = work else {
        return Some(ImportEta::Unknown);
    };
    let cost = day * left;
    let (start, cutoff) = if running {
        (now, shifted(next, MARGIN, false))
    } else {
        (next, shifted(next, WINDOW, true))
    };
    let finish = shifted(start, cost, true);
    Some(if finish <= cutoff {
        ImportEta::At { at: finish }
    } else {
        let windows = cost.as_secs().div_ceil(WINDOW.as_secs());
        ImportEta::ImportDays {
            days: u32::try_from(windows).unwrap_or(u32::MAX),
        }
    })
}

/// The earlier job, in the current window, that is running, stopped or paused
/// with work left, so the chain cannot have reached `job` first. A skipped job
/// does not stop the chain.
fn blocker(
    jobs: &ImportStatus,
    loaded: &Loaded,
    job: ImportJob,
    next: Timestamp,
) -> Option<ImportJob> {
    let since = shifted(next, DAY, false);
    ImportJob::ALL
        .into_iter()
        .take_while(|earlier| *earlier != job)
        .find(|earlier| {
            let work_left =
                *earlier != ImportJob::DailyPull && !matches!(loaded.work(*earlier), Work::Nothing);
            latest(jobs, *earlier)
                .is_some_and(|attempt| attempt.started >= since && holds_up(attempt, work_left))
        })
}

fn holds_up(attempt: &ImportAttempt, work_left: bool) -> bool {
    attempt.ended.is_none_or(|end| match end.outcome {
        AttemptOutcome::Failed(reason) => reason != ImportFailure::ProgressUnreadable,
        AttemptOutcome::Paused => work_left,
        AttemptOutcome::Done => false,
    })
}

fn latest(jobs: &ImportStatus, job: ImportJob) -> Option<&ImportAttempt> {
    jobs.jobs.get(&job).and_then(|status| status.latest())
}

fn shifted(at: Timestamp, span: Duration, forward: bool) -> Timestamp {
    let micros = i64::try_from(span.as_micros()).unwrap_or(i64::MAX);
    Timestamp::from_micros(if forward {
        at.as_micros().saturating_add(micros)
    } else {
        at.as_micros().saturating_sub(micros)
    })
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportEta, ImportFailure,
        ImportJob, ImportProgress, ImportStatus, Timestamp,
    };

    use crate::test_archive::{MemoryBackfill, MemoryCursor, ScriptedSource};
    use crate::{ActivityCursor, BackfillCursor};

    use super::{DAY, ImportCursors};

    const NOW: i64 = 1_000_000_000_000_000;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_micros(NOW + seconds * 1_000_000)
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("date")
    }

    fn days(next: NaiveDate, last: NaiveDate) -> BackfillCursor {
        BackfillCursor { next, last }
    }

    fn pending(left: u32) -> BackfillCursor {
        let next = date(2026, 1, 1);
        days(next, next + chrono::Days::new(u64::from(left) - 1))
    }

    fn finished() -> BackfillCursor {
        days(date(2026, 1, 2), date(2026, 1, 1))
    }

    #[derive(Default)]
    struct Stores {
        gap: MemoryBackfill,
        yearly: MemoryBackfill,
        archive: MemoryBackfill,
        activities: MemoryCursor,
    }

    impl Stores {
        /// Every job's work is done.
        fn quiet() -> Self {
            Self {
                gap: MemoryBackfill::holding(finished()),
                yearly: MemoryBackfill::holding(finished()),
                archive: MemoryBackfill::holding(finished()),
                activities: MemoryCursor::holding(ActivityCursor {
                    done: true,
                    ..ActivityCursor::fresh()
                }),
            }
        }

        fn status(&self, jobs: ImportStatus) -> ImportStatus {
            ImportCursors {
                gap: &self.gap,
                yearly: &self.yearly,
                activities: &self.activities,
                archive: &self.archive,
            }
            .status::<ScriptedSource>(jobs, at(0))
        }
    }

    fn running(started: i64) -> ImportAttempt {
        ImportAttempt {
            started: at(started),
            ended: None,
        }
    }

    fn ended(started: i64, outcome: AttemptOutcome) -> ImportAttempt {
        ImportAttempt {
            started: at(started),
            ended: Some(AttemptEnd {
                at: at(started + 1),
                outcome,
                counts: ImportCounts::default(),
            }),
        }
    }

    fn jobs_with(records: &[(ImportJob, ImportAttempt)]) -> ImportStatus {
        let mut jobs = ImportStatus {
            next_pull: Some(at(DAY.as_secs().try_into().expect("small"))),
            ..ImportStatus::default()
        };
        for (job, attempt) in records {
            jobs.record(*job, *attempt, at(0));
        }
        jobs
    }

    fn eta(status: &ImportStatus, job: ImportJob) -> Option<ImportEta> {
        status.jobs[&job].eta
    }

    #[test]
    fn a_fresh_yearly_cursor_gives_days_done_and_left() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(days(date(2025, 11, 15), date(2026, 11, 13))),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[]));
        assert_eq!(
            status.jobs[&ImportJob::HealthHistory].progress,
            Some(ImportProgress::Days {
                done_through: date(2025, 11, 14),
                left: 364,
            })
        );
    }

    #[test]
    fn a_done_cursor_has_no_work_and_no_estimate() {
        let status = Stores::quiet().status(jobs_with(&[(ImportJob::HealthHistory, running(0))]));
        let job = &status.jobs[&ImportJob::HealthHistory];
        assert_eq!(
            job.progress,
            Some(ImportProgress::Days {
                done_through: date(2026, 1, 1),
                left: 0,
            })
        );
        assert_eq!(job.eta, None);
        assert_eq!(eta(&status, ImportJob::Workouts), None);
    }

    #[test]
    fn the_running_job_gets_a_finish_time_from_its_per_day_cost() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(245)),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, running(0))]));
        assert_eq!(
            eta(&status, ImportJob::HealthHistory),
            Some(ImportEta::At { at: at(245 * 62) })
        );
    }

    #[test]
    fn an_archive_day_costs_its_paced_documents() {
        let stores = Stores {
            archive: MemoryBackfill::holding(pending(10)),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[(ImportJob::DailyArchive, running(0))]));
        assert_eq!(
            eta(&status, ImportJob::DailyArchive),
            Some(ImportEta::At { at: at(10 * 69) })
        );
    }

    #[test]
    fn work_past_the_window_counts_further_days_rounded_up() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(1500)),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, running(0))]));
        assert_eq!(
            eta(&status, ImportJob::HealthHistory),
            Some(ImportEta::ImportDays { days: 2 })
        );
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(100)),
            ..Stores::quiet()
        };
        let mut jobs = jobs_with(&[(ImportJob::HealthHistory, running(0))]);
        jobs.next_pull = Some(at(6_000));
        assert_eq!(
            eta(&stores.status(jobs), ImportJob::HealthHistory),
            Some(ImportEta::ImportDays { days: 1 })
        );
    }

    #[test]
    fn a_job_behind_a_running_one_waits_for_it() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(5)),
            archive: MemoryBackfill::holding(pending(5)),
            activities: MemoryCursor::holding(ActivityCursor::fresh()),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, running(0))]));
        let waits = Some(ImportEta::WaitsFor {
            job: ImportJob::HealthHistory,
        });
        assert_eq!(eta(&status, ImportJob::Workouts), waits);
        assert_eq!(eta(&status, ImportJob::DailyArchive), waits);
    }

    #[test]
    fn a_job_behind_a_stopped_one_waits_and_the_stopped_one_is_unknown() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(5)),
            archive: MemoryBackfill::holding(pending(5)),
            ..Stores::quiet()
        };
        let stopped = ended(0, AttemptOutcome::Failed(ImportFailure::Throttled));
        let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, stopped)]));
        assert_eq!(
            eta(&status, ImportJob::DailyArchive),
            Some(ImportEta::WaitsFor {
                job: ImportJob::HealthHistory
            })
        );
        assert_eq!(
            eta(&status, ImportJob::HealthHistory),
            Some(ImportEta::Unknown)
        );
    }

    #[test]
    fn a_failed_daily_pull_holds_up_every_job_with_work() {
        let stores = Stores {
            archive: MemoryBackfill::holding(pending(5)),
            ..Stores::quiet()
        };
        let failed = ended(0, AttemptOutcome::Failed(ImportFailure::Unreachable));
        let status = stores.status(jobs_with(&[(ImportJob::DailyPull, failed)]));
        assert_eq!(
            eta(&status, ImportJob::DailyArchive),
            Some(ImportEta::WaitsFor {
                job: ImportJob::DailyPull
            })
        );
    }

    #[test]
    fn nothing_else_proves_a_dependency() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(5)),
            archive: MemoryBackfill::holding(pending(5)),
            ..Stores::quiet()
        };
        let skipped = ended(0, AttemptOutcome::Failed(ImportFailure::ProgressUnreadable));
        let old = ended(
            -2 * 86_400,
            AttemptOutcome::Failed(ImportFailure::Throttled),
        );
        for earlier in [skipped, old] {
            let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, earlier)]));
            assert_eq!(
                eta(&status, ImportJob::DailyArchive),
                Some(ImportEta::Unknown)
            );
        }
    }

    fn paused(started: i64) -> ImportAttempt {
        ended(started, AttemptOutcome::Paused)
    }

    #[test]
    fn a_paused_job_with_work_left_is_estimated_from_the_next_pull() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(245)),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, paused(0))]));
        assert_eq!(
            eta(&status, ImportJob::HealthHistory),
            Some(ImportEta::At {
                at: at(86_400 + 245 * 62)
            })
        );
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(1500)),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, paused(0))]));
        assert_eq!(
            eta(&status, ImportJob::HealthHistory),
            Some(ImportEta::ImportDays { days: 2 })
        );
    }

    #[test]
    fn a_job_behind_a_paused_one_waits_for_it() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(5)),
            archive: MemoryBackfill::holding(pending(5)),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[
            (ImportJob::HealthHistory, paused(0)),
            (ImportJob::DailyArchive, paused(10)),
        ]));
        assert_eq!(
            eta(&status, ImportJob::DailyArchive),
            Some(ImportEta::WaitsFor {
                job: ImportJob::HealthHistory
            })
        );
    }

    #[test]
    fn a_paused_job_with_no_work_left_holds_nothing_up() {
        let stores = Stores {
            archive: MemoryBackfill::holding(pending(5)),
            ..Stores::quiet()
        };
        let status = stores.status(jobs_with(&[
            (ImportJob::HealthHistory, paused(0)),
            (ImportJob::DailyArchive, paused(10)),
        ]));
        assert_eq!(eta(&status, ImportJob::HealthHistory), None);
        assert_eq!(
            eta(&status, ImportJob::DailyArchive),
            Some(ImportEta::At {
                at: at(86_400 + 5 * 69)
            })
        );
    }

    #[test]
    fn a_missing_or_unreadable_cursor_has_no_progress_and_an_unknown_finish() {
        let missing = Stores {
            yearly: MemoryBackfill::default(),
            ..Stores::quiet()
        };
        let unreadable = Stores {
            yearly: MemoryBackfill {
                load_fails: true,
                ..MemoryBackfill::default()
            },
            ..Stores::quiet()
        };
        for stores in [missing, unreadable] {
            let status = stores.status(jobs_with(&[(ImportJob::HealthHistory, running(0))]));
            let job = &status.jobs[&ImportJob::HealthHistory];
            assert_eq!(job.progress, None);
            assert_eq!(job.eta, Some(ImportEta::Unknown));
        }
    }

    #[test]
    fn the_workout_walk_has_no_total_so_its_finish_is_unknown() {
        let walking = Stores {
            activities: MemoryCursor::holding(ActivityCursor::fresh()),
            ..Stores::quiet()
        };
        let status = walking.status(jobs_with(&[(ImportJob::Workouts, running(0))]));
        assert_eq!(
            status.jobs[&ImportJob::Workouts].progress,
            Some(ImportProgress::Workouts { complete: false })
        );
        assert_eq!(eta(&status, ImportJob::Workouts), Some(ImportEta::Unknown));
    }

    #[test]
    fn a_finished_history_with_a_catch_up_pending_is_unknown() {
        let catching_up = Stores {
            activities: MemoryCursor::holding(ActivityCursor {
                done: true,
                catch_up: Some(50),
                ..ActivityCursor::fresh()
            }),
            ..Stores::quiet()
        };
        let status = catching_up.status(jobs_with(&[(ImportJob::Workouts, running(0))]));
        assert_eq!(
            status.jobs[&ImportJob::Workouts].progress,
            Some(ImportProgress::Workouts { complete: true })
        );
        assert_eq!(eta(&status, ImportJob::Workouts), Some(ImportEta::Unknown));
    }

    #[test]
    fn without_a_next_pull_there_is_progress_but_no_estimate() {
        let stores = Stores {
            yearly: MemoryBackfill::holding(pending(5)),
            ..Stores::quiet()
        };
        let mut jobs = jobs_with(&[(ImportJob::HealthHistory, running(0))]);
        jobs.next_pull = None;
        let status = stores.status(jobs);
        let job = &status.jobs[&ImportJob::HealthHistory];
        assert!(job.progress.is_some());
        assert_eq!(job.eta, None);
        assert_eq!(status.next_pull, None);
    }

    #[test]
    fn the_next_pull_and_the_daily_pull_pass_through_untouched() {
        let jobs = jobs_with(&[(ImportJob::DailyPull, running(0))]);
        let next = jobs.next_pull;
        let status = Stores::quiet().status(jobs);
        assert_eq!(status.next_pull, next);
        let daily = &status.jobs[&ImportJob::DailyPull];
        assert_eq!((daily.progress, daily.eta), (None, None));
    }
}
