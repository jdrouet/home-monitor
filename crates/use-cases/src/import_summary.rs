use domain::{
    AttemptOutcome, ImportCounts, ImportEta, ImportJob, ImportJobStatus, ImportStatus, Timestamp,
};

use crate::import_copy::{counts_text, failure_text, progress_text};

/// Where one Garmin job stands for the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportState {
    NotRun,
    Running,
    /// The latest attempt finished with nothing failed.
    Worked,
    /// The latest attempt finished but some days failed.
    Partial,
    /// The latest attempt ran out of time and continues after the next pull.
    CatchingUp,
    /// The latest attempt failed and a retry is planned.
    Retrying,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLine {
    pub job: ImportJob,
    pub state: ImportState,
    pub started: Option<Timestamp>,
    pub ended: Option<Timestamp>,
    /// What the latest ended attempt did; `None` when it did nothing.
    pub counts: Option<String>,
    pub failure: Option<&'static str>,
    pub progress: Option<String>,
    pub eta: Option<ImportEta>,
    /// The scheduled pull a job that gave up waits for.
    pub retry_at: Option<Timestamp>,
    /// Retries that ran before the latest attempt in this window.
    pub retries: usize,
    /// The instant of a planned retry.
    pub next_retry: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub next_pull: Option<Timestamp>,
    /// One line per job, in run order.
    pub lines: Vec<ImportLine>,
}

pub fn build_import_summary(status: &ImportStatus) -> ImportSummary {
    ImportSummary {
        next_pull: status.next_pull,
        lines: ImportJob::ALL
            .into_iter()
            .map(|job| line(job, status.jobs.get(&job)))
            .collect(),
    }
}

fn line(job: ImportJob, status: Option<&ImportJobStatus>) -> ImportLine {
    let latest = status.and_then(|status| status.attempts.last());
    let end = latest.and_then(|attempt| attempt.ended);
    let next_retry = status.and_then(|status| status.next_retry);
    let counts = window_counts(status);
    let state = match end.map(|end| end.outcome) {
        _ if latest.is_none() => ImportState::NotRun,
        None => ImportState::Running,
        Some(AttemptOutcome::Done) if counts.failed > 0 => ImportState::Partial,
        Some(AttemptOutcome::Done) => ImportState::Worked,
        Some(AttemptOutcome::Paused | AttemptOutcome::Capped) => ImportState::CatchingUp,
        Some(AttemptOutcome::Failed(_)) if next_retry.is_some() => ImportState::Retrying,
        Some(AttemptOutcome::Failed(_)) => ImportState::Failed,
    };
    ImportLine {
        job,
        state,
        started: latest.map(|attempt| attempt.started),
        ended: end.map(|end| end.at),
        counts: end.and_then(|_| counts_text(job, counts)),
        failure: match end.map(|end| end.outcome) {
            Some(AttemptOutcome::Failed(reason)) => Some(failure_text(reason)),
            _ => None,
        },
        progress: status.and_then(|status| status.progress).map(progress_text),
        eta: status.and_then(|status| status.eta),
        retry_at: status.and_then(|status| status.retry_at),
        retries: status.map_or(0, ImportJobStatus::retries),
        next_retry,
    }
}

/// What every ended attempt of the window did, so a day skipped before a
/// retry is not forgotten when the retry is clean.
fn window_counts(status: Option<&ImportJobStatus>) -> ImportCounts {
    status
        .into_iter()
        .flat_map(|status| &status.attempts)
        .filter_map(|attempt| attempt.ended)
        .fold(ImportCounts::default(), |sum, end| ImportCounts {
            stored: sum.stored + end.counts.stored,
            empty: sum.empty + end.counts.empty,
            failed: sum.failed + end.counts.failed,
        })
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportEta, ImportFailure,
        ImportJob, ImportProgress, ImportStatus, Timestamp,
    };

    use crate::test_archive::stored;

    use super::{ImportState, build_import_summary};

    fn at(micros: i64) -> Timestamp {
        Timestamp::from_micros(micros)
    }

    fn ended(started: i64, outcome: AttemptOutcome, counts: ImportCounts) -> ImportAttempt {
        ImportAttempt {
            started: at(started),
            ended: Some(AttemptEnd {
                at: at(started + 1),
                outcome,
                counts,
            }),
        }
    }

    fn summary_of(job: ImportJob, attempt: ImportAttempt) -> super::ImportLine {
        let mut status = ImportStatus::default();
        status.record(job, attempt, at(100), None);
        build_import_summary(&status)
            .lines
            .into_iter()
            .find(|line| line.job == job)
            .expect("a line for every job")
    }

    #[test]
    fn there_is_one_line_per_job_in_run_order_and_missing_jobs_have_not_run() {
        let summary = build_import_summary(&ImportStatus::default());
        let jobs: Vec<_> = summary.lines.iter().map(|line| line.job).collect();
        assert_eq!(jobs, ImportJob::ALL);
        assert!(
            summary
                .lines
                .iter()
                .all(|line| line.state == ImportState::NotRun && line.started.is_none())
        );
        assert_eq!(summary.next_pull, None);
    }

    #[test]
    fn each_outcome_gives_its_state() {
        let running = ImportAttempt {
            started: at(1),
            ended: None,
        };
        let line = summary_of(ImportJob::Workouts, running);
        assert_eq!(
            (line.state, line.started, line.ended),
            (ImportState::Running, Some(at(1)), None)
        );
        let done = summary_of(
            ImportJob::Workouts,
            ended(1, AttemptOutcome::Done, stored(3)),
        );
        assert_eq!(done.state, ImportState::Worked);
        assert_eq!(done.counts.as_deref(), Some("3 new workouts archived"));
        let partial = summary_of(
            ImportJob::MissedDays,
            ended(
                1,
                AttemptOutcome::Done,
                ImportCounts {
                    stored: 1,
                    failed: 2,
                    ..ImportCounts::default()
                },
            ),
        );
        assert_eq!(partial.state, ImportState::Partial);
        assert_eq!(partial.counts.as_deref(), Some("1 stored, 2 skipped"));
        let paused = summary_of(
            ImportJob::HealthHistory,
            ended(1, AttemptOutcome::Paused, stored(9)),
        );
        assert_eq!(paused.state, ImportState::CatchingUp);
        assert_eq!(paused.ended, Some(at(2)));
    }

    #[test]
    fn a_failure_carries_its_reason_counts_and_retry_time() {
        let failed = AttemptOutcome::Failed(ImportFailure::GarminDown);
        let line = summary_of(ImportJob::Workouts, ended(1, failed, stored(2)));
        assert_eq!(line.state, ImportState::Failed);
        assert_eq!(line.failure, Some("Garmin had a server error."));
        assert_eq!(line.retry_at, Some(at(100)));
        assert!(line.counts.is_some());
        let quiet = summary_of(
            ImportJob::Workouts,
            ended(1, failed, ImportCounts::default()),
        );
        assert_eq!(quiet.counts, None);
    }

    #[test]
    fn progress_and_the_finish_estimate_pass_through() {
        let mut status = ImportStatus {
            next_pull: Some(at(7)),
            ..ImportStatus::default()
        };
        let job = status.jobs.entry(ImportJob::HealthHistory).or_default();
        job.progress = Some(ImportProgress::Days {
            done_through: NaiveDate::from_ymd_opt(2026, 3, 11).expect("date"),
            left: 245,
        });
        job.eta = Some(ImportEta::ImportDays { days: 2 });
        let summary = build_import_summary(&status);
        let line = &summary.lines[2];
        assert_eq!(
            line.progress.as_deref(),
            Some("Up to 2026-03-11 · 245 days left")
        );
        assert_eq!(line.eta, Some(ImportEta::ImportDays { days: 2 }));
        assert_eq!(line.state, ImportState::NotRun);
        assert_eq!(summary.next_pull, Some(at(7)));
    }

    #[test]
    fn a_planned_retry_is_retrying_with_its_count() {
        let failed = AttemptOutcome::Failed(ImportFailure::GarminDown);
        let mut status = ImportStatus::default();
        status.record(
            ImportJob::Workouts,
            ended(1, failed, ImportCounts::default()),
            at(100),
            Some(at(50)),
        );
        let line = build_import_summary(&status).lines.remove(3);
        assert_eq!(line.state, ImportState::Retrying);
        assert_eq!((line.retries, line.next_retry), (0, Some(at(50))));
        assert_eq!(line.failure, Some("Garmin had a server error."));
        assert_eq!(line.retry_at, None);
    }

    #[test]
    fn a_success_on_retry_2_is_worked_with_two_retries() {
        let failed = AttemptOutcome::Failed(ImportFailure::Unreachable);
        let mut status = ImportStatus::default();
        for (started, outcome, retry) in [
            (1, failed, Some(at(50))),
            (50, failed, Some(at(90))),
            (90, AttemptOutcome::Done, None),
        ] {
            status.record(
                ImportJob::Workouts,
                ended(started, outcome, ImportCounts::default()),
                at(100),
                retry,
            );
        }
        let line = build_import_summary(&status).lines.remove(3);
        assert_eq!((line.state, line.retries), (ImportState::Worked, 2));
    }

    #[test]
    fn a_day_skipped_before_a_clean_retry_keeps_the_job_partial() {
        let skipped = ImportCounts {
            stored: 2,
            failed: 1,
            ..ImportCounts::default()
        };
        let mut status = ImportStatus::default();
        status.record(
            ImportJob::MissedDays,
            ended(
                1,
                AttemptOutcome::Failed(ImportFailure::Unreachable),
                skipped,
            ),
            at(100),
            Some(at(50)),
        );
        status.record(
            ImportJob::MissedDays,
            ended(50, AttemptOutcome::Done, stored(3)),
            at(100),
            None,
        );
        let line = build_import_summary(&status).lines.remove(1);
        assert_eq!(line.state, ImportState::Partial);
        assert_eq!(line.counts.as_deref(), Some("5 stored, 1 skipped"));
    }

    #[test]
    fn a_give_up_after_retries_is_failed_with_three_retries() {
        let failed = AttemptOutcome::Failed(ImportFailure::GarminDown);
        let mut status = ImportStatus::default();
        for (started, retry) in [
            (1, Some(at(10))),
            (10, Some(at(20))),
            (20, Some(at(30))),
            (30, None),
        ] {
            let attempt = ended(started, failed, ImportCounts::default());
            status.record(ImportJob::Workouts, attempt, at(100), retry);
        }
        let line = build_import_summary(&status).lines.remove(3);
        assert_eq!((line.state, line.retries), (ImportState::Failed, 3));
        assert_eq!(line.retry_at, Some(at(100)));
    }
}
