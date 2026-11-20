use chrono::DateTime;
use chrono_tz::Europe::Paris;
use domain::{ImportFailure, ImportJob, Timestamp};

use crate::Notification;
use crate::import_copy::{failure_text, job_name};

/// The give-up of `job`, which tries again at `retry_at`, shown in Paris time.
pub(crate) fn import_notification(
    job: ImportJob,
    failure: ImportFailure,
    retry_at: Timestamp,
) -> Notification {
    let when =
        DateTime::from_timestamp_micros(retry_at.as_micros()).map_or_else(String::new, |utc| {
            let paris = utc.with_timezone(&Paris);
            format!(", {} Paris time", paris.format("%Y-%m-%d %H:%M"))
        });
    Notification {
        title: format!("Garmin import stopped: {}", job_name(job)),
        message: format!(
            "{} It tries again at the next daily pull{when}.",
            failure_text(failure)
        ),
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use domain::{ImportFailure, ImportJob, Timestamp};

    use super::import_notification;

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    #[test]
    fn it_names_the_job_the_reason_and_the_paris_time_of_the_next_try() {
        let notification = import_notification(
            ImportJob::Workouts,
            ImportFailure::GarminDown,
            at("2026-07-16T08:00:00Z"),
        );
        assert_eq!(notification.title, "Garmin import stopped: Workout archive");
        assert_eq!(
            notification.message,
            "Garmin had a server error. It tries again at the next daily pull, 2026-07-16 10:00 Paris time."
        );
    }

    #[test]
    fn an_instant_out_of_range_still_gives_a_sentence() {
        let notification = import_notification(
            ImportJob::DailyPull,
            ImportFailure::Unreachable,
            Timestamp::from_micros(i64::MAX),
        );
        assert!(notification.message.ends_with("at the next daily pull."));
    }
}
