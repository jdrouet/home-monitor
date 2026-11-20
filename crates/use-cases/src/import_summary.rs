use domain::{AttemptOutcome, ImportEta, ImportJob, ImportJobStatus, ImportStatus, Timestamp};

use crate::import_copy::{counts_text, failure_text, job_name, no_counts_text, progress_text};

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
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLine {
    pub job: ImportJob,
    pub name: &'static str,
    pub state: ImportState,
    pub started: Option<Timestamp>,
    pub ended: Option<Timestamp>,
    /// What the latest ended attempt did, or that it did nothing.
    pub counts: String,
    /// Whether `counts` says something was done.
    pub counted: bool,
    pub failure: Option<&'static str>,
    pub progress: Option<String>,
    pub eta: Option<ImportEta>,
    /// The scheduled pull a failed job waits for.
    pub retry_at: Option<Timestamp>,
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
    let latest = status.and_then(ImportJobStatus::latest);
    let end = latest.and_then(|attempt| attempt.ended);
    let state = match end.map(|end| end.outcome) {
        _ if latest.is_none() => ImportState::NotRun,
        None => ImportState::Running,
        Some(AttemptOutcome::Done) if end.is_some_and(|end| end.counts.failed > 0) => {
            ImportState::Partial
        }
        Some(AttemptOutcome::Done) => ImportState::Worked,
        Some(AttemptOutcome::Paused | AttemptOutcome::Capped) => ImportState::CatchingUp,
        Some(AttemptOutcome::Failed(_)) => ImportState::Failed,
    };
    let counts = end.and_then(|end| counts_text(job, end.counts));
    ImportLine {
        job,
        name: job_name(job),
        state,
        started: latest.map(|attempt| attempt.started),
        ended: end.map(|end| end.at),
        counted: counts.is_some(),
        counts: counts.unwrap_or_else(|| no_counts_text(job).to_owned()),
        failure: match end.map(|end| end.outcome) {
            Some(AttemptOutcome::Failed(reason)) => Some(failure_text(reason)),
            _ => None,
        },
        progress: status.and_then(|status| status.progress).map(progress_text),
        eta: status.and_then(|status| status.eta),
        retry_at: status.and_then(|status| status.retry_at),
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportEta, ImportFailure,
        ImportJob, ImportProgress, ImportStatus, Timestamp,
    };

    use super::{ImportState, build_import_summary};

    fn at(micros: i64) -> Timestamp {
        Timestamp::from_micros(micros)
    }

    fn ended(outcome: AttemptOutcome, counts: ImportCounts) -> ImportAttempt {
        ImportAttempt {
            started: at(1),
            ended: Some(AttemptEnd {
                at: at(2),
                outcome,
                counts,
            }),
        }
    }

    fn stored(count: u32) -> ImportCounts {
        ImportCounts {
            stored: count,
            ..ImportCounts::default()
        }
    }

    fn summary_of(job: ImportJob, attempt: ImportAttempt) -> super::ImportLine {
        let mut status = ImportStatus::default();
        status.record(job, attempt, at(100));
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
        let done = summary_of(ImportJob::Workouts, ended(AttemptOutcome::Done, stored(3)));
        assert_eq!(done.state, ImportState::Worked);
        assert_eq!(done.counts, "3 new workouts archived");
        assert!(done.counted);
        let partial = summary_of(
            ImportJob::MissedDays,
            ended(
                AttemptOutcome::Done,
                ImportCounts {
                    stored: 1,
                    failed: 2,
                    ..ImportCounts::default()
                },
            ),
        );
        assert_eq!(partial.state, ImportState::Partial);
        assert_eq!(partial.counts, "1 stored, 2 skipped");
        let paused = summary_of(
            ImportJob::HealthHistory,
            ended(AttemptOutcome::Paused, stored(9)),
        );
        assert_eq!(paused.state, ImportState::CatchingUp);
        assert_eq!(paused.ended, Some(at(2)));
    }

    #[test]
    fn a_failure_carries_its_reason_counts_and_retry_time() {
        let failed = AttemptOutcome::Failed(ImportFailure::GarminDown);
        let line = summary_of(ImportJob::Workouts, ended(failed, stored(2)));
        assert_eq!(line.state, ImportState::Failed);
        assert_eq!(line.failure, Some("Garmin had a server error."));
        assert_eq!(line.retry_at, Some(at(100)));
        assert!(line.counted);
        let quiet = summary_of(ImportJob::Workouts, ended(failed, ImportCounts::default()));
        assert!(!quiet.counted);
        assert_eq!(quiet.counts, "no new workouts");
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
}
