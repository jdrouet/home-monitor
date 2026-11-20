use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{AttemptOutcome, ImportAttempt, ImportJob, ImportJobStatus, Timestamp};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportStatus {
    /// The scheduled next pull, set by the pull loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_pull: Option<Timestamp>,
    pub jobs: BTreeMap<ImportJob, ImportJobStatus>,
}

impl ImportStatus {
    /// A running attempt replaces the job's attempts and clears `retry_at`.
    /// An ended one replaces the latest attempt with the same start. A failure
    /// that ends before `deadline` sets `retry_at` to it; any other end clears it.
    pub fn record(&mut self, job: ImportJob, attempt: ImportAttempt, deadline: Timestamp) {
        let status = self.jobs.entry(job).or_default();
        status.retry_at = attempt
            .ended
            .filter(|end| matches!(end.outcome, AttemptOutcome::Failed(_)) && end.at < deadline)
            .map(|_| deadline);
        match status.attempts.last_mut() {
            Some(last) if attempt.ended.is_some() && last.started == attempt.started => {
                *last = attempt;
            }
            _ => status.attempts = vec![attempt],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure, ImportJob,
        ImportJobStatus, Timestamp,
    };

    use super::ImportStatus;

    const JOB: ImportJob = ImportJob::Workouts;

    fn at(micros: i64) -> Timestamp {
        Timestamp::from_micros(micros)
    }

    fn running(started: i64) -> ImportAttempt {
        ImportAttempt {
            started: at(started),
            ended: None,
        }
    }

    fn ended(started: i64, end: i64, outcome: AttemptOutcome) -> ImportAttempt {
        ImportAttempt {
            started: at(started),
            ended: Some(AttemptEnd {
                at: at(end),
                outcome,
                counts: ImportCounts::default(),
            }),
        }
    }

    fn failed() -> AttemptOutcome {
        AttemptOutcome::Failed(ImportFailure::GarminDown)
    }

    fn job(status: &ImportStatus) -> &ImportJobStatus {
        &status.jobs[&JOB]
    }

    #[test]
    fn start_then_done_keeps_one_attempt_and_no_retry() {
        let mut status = ImportStatus::default();
        status.record(JOB, running(1), at(100));
        status.record(JOB, ended(1, 2, AttemptOutcome::Done), at(100));
        assert_eq!(
            job(&status).attempts,
            vec![ended(1, 2, AttemptOutcome::Done)]
        );
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn a_failure_before_the_deadline_sets_retry_at() {
        let mut status = ImportStatus::default();
        status.record(JOB, running(1), at(100));
        status.record(JOB, ended(1, 2, failed()), at(100));
        assert_eq!(job(&status).retry_at, Some(at(100)));
    }

    #[test]
    fn a_failure_at_or_after_the_deadline_sets_no_retry() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 100, failed()), at(100));
        assert_eq!(job(&status).retry_at, None);
        status.record(JOB, ended(1, 101, failed()), at(100));
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn a_new_start_clears_retry_at_and_older_attempts() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100));
        status.record(JOB, running(3), at(200));
        assert_eq!(job(&status).attempts, vec![running(3)]);
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn a_paused_end_clears_retry_at() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100));
        status.record(JOB, ended(3, 4, AttemptOutcome::Paused), at(100));
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn an_end_without_a_matching_start_becomes_the_only_attempt() {
        let mut status = ImportStatus::default();
        status.record(JOB, running(1), at(100));
        status.record(JOB, ended(5, 6, AttemptOutcome::Done), at(100));
        assert_eq!(
            job(&status).attempts,
            vec![ended(5, 6, AttemptOutcome::Done)]
        );
    }

    #[test]
    fn jobs_are_recorded_independently() {
        let mut status = ImportStatus::default();
        status.record(ImportJob::DailyPull, running(1), at(100));
        status.record(JOB, running(2), at(100));
        assert_eq!(status.jobs.len(), 2);
    }
}
