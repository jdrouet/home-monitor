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
    /// An ended attempt replaces the latest one with the same start; a new start
    /// appends when a retry was pending, else replaces the attempts. `retry_at`
    /// is set only on a give-up, so it and `next_retry` are never both set.
    pub fn record(
        &mut self,
        job: ImportJob,
        attempt: ImportAttempt,
        deadline: Timestamp,
        next_retry: Option<Timestamp>,
    ) {
        let status = self.jobs.entry(job).or_default();
        let retrying = status.next_retry.take().is_some();
        status.next_retry = attempt.ended.and(next_retry);
        status.retry_at = attempt
            .ended
            .filter(|end| {
                matches!(end.outcome, AttemptOutcome::Failed(_))
                    && next_retry.is_none()
                    && end.at < deadline
            })
            .map(|_| deadline);
        match status.attempts.last_mut() {
            Some(last) if attempt.ended.is_some() && last.started == attempt.started => {
                *last = attempt;
            }
            _ if retrying => status.attempts.push(attempt),
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
        status.record(JOB, running(1), at(100), None);
        status.record(JOB, ended(1, 2, AttemptOutcome::Done), at(100), None);
        assert_eq!(
            job(&status).attempts,
            vec![ended(1, 2, AttemptOutcome::Done)]
        );
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn a_failure_before_the_deadline_sets_retry_at() {
        let mut status = ImportStatus::default();
        status.record(JOB, running(1), at(100), None);
        status.record(JOB, ended(1, 2, failed()), at(100), None);
        assert_eq!(job(&status).retry_at, Some(at(100)));
    }

    #[test]
    fn a_failure_at_or_after_the_deadline_sets_no_retry() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 100, failed()), at(100), None);
        assert_eq!(job(&status).retry_at, None);
        status.record(JOB, ended(1, 101, failed()), at(100), None);
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn a_new_start_clears_retry_at_and_older_attempts() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100), None);
        status.record(JOB, running(3), at(200), None);
        assert_eq!(job(&status).attempts, vec![running(3)]);
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn a_paused_end_clears_retry_at() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100), None);
        status.record(JOB, ended(3, 4, AttemptOutcome::Paused), at(100), None);
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn an_end_without_a_matching_start_becomes_the_only_attempt() {
        let mut status = ImportStatus::default();
        status.record(JOB, running(1), at(100), None);
        status.record(JOB, ended(5, 6, AttemptOutcome::Done), at(100), None);
        assert_eq!(
            job(&status).attempts,
            vec![ended(5, 6, AttemptOutcome::Done)]
        );
    }

    #[test]
    fn jobs_are_recorded_independently() {
        let mut status = ImportStatus::default();
        status.record(ImportJob::DailyPull, running(1), at(100), None);
        status.record(JOB, running(2), at(100), None);
        assert_eq!(status.jobs.len(), 2);
    }

    #[test]
    fn a_planned_retry_sets_next_retry_and_no_retry_at() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100), Some(at(50)));
        assert_eq!(job(&status).next_retry, Some(at(50)));
        assert_eq!(job(&status).retry_at, None);
    }

    #[test]
    fn a_retry_appends_its_attempt_and_clears_next_retry() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100), Some(at(50)));
        status.record(JOB, running(50), at(100), None);
        assert_eq!(job(&status).next_retry, None);
        status.record(JOB, ended(50, 51, AttemptOutcome::Done), at(100), None);
        assert_eq!(
            job(&status).attempts,
            vec![ended(1, 2, failed()), ended(50, 51, AttemptOutcome::Done)]
        );
    }

    #[test]
    fn ended_retries_append_in_order() {
        let mut status = ImportStatus::default();
        for start in [1, 10, 20] {
            status.record(JOB, running(start), at(100), None);
            status.record(
                JOB,
                ended(start, start + 1, failed()),
                at(100),
                Some(at(90)),
            );
        }
        status.record(JOB, running(30), at(100), None);
        status.record(JOB, ended(30, 31, AttemptOutcome::Done), at(100), None);
        let job = job(&status);
        assert_eq!(job.attempts.len(), 4);
        assert_eq!(job.attempts[3], ended(30, 31, AttemptOutcome::Done));
        assert_eq!((job.retry_at, job.next_retry), (None, None));
    }

    #[test]
    fn the_last_failed_retry_sets_retry_at() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100), Some(at(50)));
        status.record(JOB, running(50), at(100), None);
        status.record(JOB, ended(50, 51, failed()), at(100), None);
        let job = job(&status);
        assert_eq!(job.retry_at, Some(at(100)));
        assert_eq!(job.next_retry, None);
        assert_eq!(job.attempts.len(), 2);
    }

    #[test]
    fn a_new_window_replaces_the_retried_attempts() {
        let mut status = ImportStatus::default();
        status.record(JOB, ended(1, 2, failed()), at(100), Some(at(50)));
        status.record(JOB, running(50), at(100), None);
        status.record(JOB, ended(50, 51, failed()), at(100), None);
        status.record(JOB, running(200), at(300), None);
        assert_eq!(job(&status).attempts, vec![running(200)]);
    }
}
