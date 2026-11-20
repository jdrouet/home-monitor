use domain::{ImportFailure, ImportJob};

use crate::Notification;
use crate::import_copy::{failure_text, job_name, retries_failed_text};

/// The give-up of `job` after `retries` failed retries, which tries again at
/// the time `when` names.
pub(crate) fn import_notification(
    job: ImportJob,
    failure: ImportFailure,
    retries: usize,
    when: &str,
) -> Notification {
    let retried = if retries == 0 {
        String::new()
    } else {
        format!(" {}", retries_failed_text(retries))
    };
    Notification {
        title: format!("Garmin import stopped: {}", job_name(job)),
        message: format!(
            "{}{retried} It tries again at the next daily pull, {when}.",
            failure_text(failure)
        ),
    }
}

#[cfg(test)]
mod tests {
    use domain::{ImportFailure, ImportJob};

    use super::import_notification;

    #[test]
    fn it_names_the_job_the_reason_and_the_time_of_the_next_try() {
        let notification = import_notification(
            ImportJob::Workouts,
            ImportFailure::GarminDown,
            0,
            "2026-07-16 10:00 Paris time",
        );
        assert_eq!(notification.title, "Garmin import stopped: Workout archive");
        assert_eq!(
            notification.message,
            "Garmin had a server error. It tries again at the next daily pull, 2026-07-16 10:00 Paris time."
        );
    }

    #[test]
    fn it_says_how_many_retries_failed() {
        let message = |retries| {
            import_notification(
                ImportJob::Workouts,
                ImportFailure::GarminDown,
                retries,
                "10:00",
            )
            .message
        };
        assert_eq!(
            message(1),
            "Garmin had a server error. 1 retry failed. It tries again at the next daily pull, 10:00."
        );
        assert_eq!(
            message(3),
            "Garmin had a server error. 3 retries failed. It tries again at the next daily pull, 10:00."
        );
    }
}
