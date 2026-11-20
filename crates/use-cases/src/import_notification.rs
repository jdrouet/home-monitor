use domain::{ImportFailure, ImportJob};

use crate::Notification;
use crate::import_copy::{failure_text, job_name};

/// The give-up of `job`, which tries again at the time `when` names.
pub(crate) fn import_notification(
    job: ImportJob,
    failure: ImportFailure,
    when: &str,
) -> Notification {
    Notification {
        title: format!("Garmin import stopped: {}", job_name(job)),
        message: format!(
            "{} It tries again at the next daily pull, {when}.",
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
            "2026-07-16 10:00 Paris time",
        );
        assert_eq!(notification.title, "Garmin import stopped: Workout archive");
        assert_eq!(
            notification.message,
            "Garmin had a server error. It tries again at the next daily pull, 2026-07-16 10:00 Paris time."
        );
    }
}
