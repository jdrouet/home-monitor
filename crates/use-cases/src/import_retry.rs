use std::time::Duration;

use domain::{ImportFailure, Timestamp};

use crate::import_window::step_fits;

/// Waits before the first, second and third retry of a failed job.
pub(crate) const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
    Duration::from_secs(60 * 60),
];

/// Retries a job gets in one window.
pub const MAX_RETRIES: usize = RETRY_DELAYS.len();

/// Garmin did not answer, or answered badly. A rate limit is not a bad answer:
/// retrying it could lengthen the block. The other reasons need the owner.
pub(crate) fn is_transient(failure: ImportFailure) -> bool {
    match failure {
        ImportFailure::Unreachable | ImportFailure::GarminDown | ImportFailure::BadAnswer => true,
        ImportFailure::LoginRejected
        | ImportFailure::Throttled
        | ImportFailure::LoginFileUnreadable
        | ImportFailure::ArchiveUnavailable
        | ImportFailure::ProgressUnreadable
        | ImportFailure::ProgressNotSaved
        | ImportFailure::PublishFailed => false,
    }
}

/// When to retry a job that stopped on `failure` at `ended` after `retries`
/// retries; `None` gives up until the next pull.
pub(crate) fn next_retry(
    failure: ImportFailure,
    retries: usize,
    ended: Timestamp,
    until: Timestamp,
) -> Option<Timestamp> {
    if !is_transient(failure) {
        return None;
    }
    let at = ended.saturating_add(*RETRY_DELAYS.get(retries)?);
    step_fits(until, at).then_some(at)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use domain::{ImportFailure, Timestamp};

    use crate::import_window::{MARGIN, STEP_SPACING};

    use super::{is_transient, next_retry};

    const ALL: [(ImportFailure, bool); 10] = [
        (ImportFailure::Unreachable, true),
        (ImportFailure::GarminDown, true),
        (ImportFailure::BadAnswer, true),
        (ImportFailure::Throttled, false),
        (ImportFailure::LoginRejected, false),
        (ImportFailure::LoginFileUnreadable, false),
        (ImportFailure::ArchiveUnavailable, false),
        (ImportFailure::ProgressUnreadable, false),
        (ImportFailure::ProgressNotSaved, false),
        (ImportFailure::PublishFailed, false),
    ];

    fn micros(duration: Duration) -> i64 {
        i64::try_from(duration.as_micros()).expect("fits")
    }

    #[test]
    fn every_reason_is_classified() {
        for (failure, transient) in ALL {
            assert_eq!(is_transient(failure), transient, "{failure:?}");
        }
    }

    #[test]
    fn next_retry_follows_the_delays_then_gives_up() {
        let ended = Timestamp::from_micros(1_000);
        let until = Timestamp::from_micros(1_000 + micros(Duration::from_secs(86_400)));
        for (retries, minutes) in [5, 15, 60].into_iter().enumerate() {
            assert_eq!(
                next_retry(ImportFailure::BadAnswer, retries, ended, until),
                Some(Timestamp::from_micros(
                    1_000 + micros(Duration::from_secs(minutes * 60))
                ))
            );
        }
        assert_eq!(next_retry(ImportFailure::BadAnswer, 3, ended, until), None);
    }

    #[test]
    fn a_retry_must_fit_a_step_before_the_deadline() {
        let ended = Timestamp::from_micros(0);
        let needed = micros(Duration::from_secs(5 * 60)) + micros(STEP_SPACING + MARGIN);
        let fits = Timestamp::from_micros(needed);
        let short = Timestamp::from_micros(needed - 1);
        assert!(next_retry(ImportFailure::GarminDown, 0, ended, fits).is_some());
        assert_eq!(next_retry(ImportFailure::GarminDown, 0, ended, short), None);
    }
}
