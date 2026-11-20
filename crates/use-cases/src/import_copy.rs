use domain::{ImportFailure, ImportJob};

/// The name the owner sees for `job`.
pub fn job_name(job: ImportJob) -> &'static str {
    match job {
        ImportJob::DailyPull => "Daily pull",
        ImportJob::MissedDays => "Missed days (last 30)",
        ImportJob::HealthHistory => "One-year health import",
        ImportJob::Workouts => "Workout archive",
        ImportJob::DailyArchive => "Daily archive",
    }
}

/// The reason shown to the owner when a job stopped.
pub fn failure_text(failure: ImportFailure) -> &'static str {
    match failure {
        ImportFailure::LoginRejected => "The Garmin login is missing or was refused.",
        ImportFailure::Throttled => "Garmin asked to slow down.",
        ImportFailure::Unreachable => "Garmin could not be reached (network error or timeout).",
        ImportFailure::GarminDown => "Garmin had a server error.",
        ImportFailure::BadAnswer => "Garmin sent an answer that could not be used.",
        ImportFailure::LoginFileUnreadable => "The saved Garmin login could not be read on the Pi.",
        ImportFailure::ArchiveUnavailable => "The archive folder could not be read or written.",
        ImportFailure::ProgressUnreadable => {
            "Its progress file is unreadable and needs attention; fix or delete it on the Pi."
        }
        ImportFailure::ProgressNotSaved => "Its progress could not be saved on the Pi.",
        ImportFailure::PublishFailed => "The data could not be sent to the store.",
    }
}

#[cfg(test)]
mod tests {
    use domain::{ImportFailure, ImportJob};

    use super::{failure_text, job_name};

    const FAILURES: [ImportFailure; 10] = [
        ImportFailure::LoginRejected,
        ImportFailure::Throttled,
        ImportFailure::Unreachable,
        ImportFailure::GarminDown,
        ImportFailure::BadAnswer,
        ImportFailure::LoginFileUnreadable,
        ImportFailure::ArchiveUnavailable,
        ImportFailure::ProgressUnreadable,
        ImportFailure::ProgressNotSaved,
        ImportFailure::PublishFailed,
    ];

    #[test]
    fn every_job_and_failure_has_copy_without_a_link() {
        for job in ImportJob::ALL {
            assert!(!job_name(job).is_empty());
        }
        for failure in FAILURES {
            let text = failure_text(failure);
            assert!(text.ends_with('.'), "{text}");
            assert!(!text.contains("http"), "{text}");
        }
    }

    #[test]
    fn the_job_names_follow_the_run_order() {
        let names: Vec<_> = ImportJob::ALL.into_iter().map(job_name).collect();
        assert_eq!(
            names,
            [
                "Daily pull",
                "Missed days (last 30)",
                "One-year health import",
                "Workout archive",
                "Daily archive",
            ]
        );
    }
}
