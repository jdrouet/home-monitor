use domain::{ImportCounts, ImportFailure, ImportJob, ImportProgress};

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

/// What an attempt did, in the job's own terms, with the zero parts left out;
/// `None` when nothing remains to say.
pub(crate) fn counts_text(job: ImportJob, counts: ImportCounts) -> Option<String> {
    let parts: Vec<(u32, &str, &str)> = match job {
        ImportJob::DailyPull => vec![
            (counts.stored, "day pulled", "days pulled"),
            (counts.empty, "empty", "empty"),
            (counts.failed, "failed", "failed"),
        ],
        ImportJob::MissedDays | ImportJob::HealthHistory => vec![
            (counts.stored, "stored", "stored"),
            (counts.empty, "empty", "empty"),
            (counts.failed, "skipped", "skipped"),
        ],
        ImportJob::Workouts => vec![(
            counts.stored,
            "new workout archived",
            "new workouts archived",
        )],
        ImportJob::DailyArchive => vec![(counts.stored, "day archived", "days archived")],
    };
    let shown: Vec<String> = parts
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, one, many)| format!("{count} {}", if count == 1 { one } else { many }))
        .collect();
    (!shown.is_empty()).then(|| shown.join(", "))
}

/// What to say when an attempt did nothing.
pub(crate) fn no_counts_text(job: ImportJob) -> &'static str {
    match job {
        ImportJob::DailyPull => "no days pulled",
        ImportJob::MissedDays | ImportJob::HealthHistory => "nothing to import",
        ImportJob::Workouts => "no new workouts",
        ImportJob::DailyArchive => "no new days archived",
    }
}

/// Where a job stands, from its cursor.
pub(crate) fn progress_text(progress: ImportProgress) -> String {
    match progress {
        ImportProgress::Days {
            done_through,
            left: 0,
        } => format!("Complete up to {done_through}"),
        ImportProgress::Days {
            done_through,
            left: 1,
        } => format!("Up to {done_through} · 1 day left"),
        ImportProgress::Days { done_through, left } => {
            format!("Up to {done_through} · {left} days left")
        }
        ImportProgress::Workouts { complete: false } => "Walking the workout history".to_owned(),
        ImportProgress::Workouts { complete: true } => "Workout history complete".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use domain::{ImportCounts, ImportFailure, ImportJob, ImportProgress};

    use super::{counts_text, failure_text, job_name, no_counts_text, progress_text};

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

    fn counts(stored: u32, empty: u32, failed: u32) -> ImportCounts {
        ImportCounts {
            stored,
            empty,
            failed,
        }
    }

    #[test]
    fn counts_leave_out_zero_parts_and_follow_the_singular() {
        let text = |job, counts| counts_text(job, counts);
        assert_eq!(
            text(ImportJob::DailyPull, counts(2, 0, 0)).as_deref(),
            Some("2 days pulled")
        );
        assert_eq!(
            text(ImportJob::DailyPull, counts(1, 1, 1)).as_deref(),
            Some("1 day pulled, 1 empty, 1 failed")
        );
        assert_eq!(
            text(ImportJob::HealthHistory, counts(300, 60, 4)).as_deref(),
            Some("300 stored, 60 empty, 4 skipped")
        );
        assert_eq!(
            text(ImportJob::MissedDays, counts(0, 0, 2)).as_deref(),
            Some("2 skipped")
        );
        assert_eq!(
            text(ImportJob::Workouts, counts(1, 0, 0)).as_deref(),
            Some("1 new workout archived")
        );
        assert_eq!(
            text(ImportJob::Workouts, counts(3, 0, 0)).as_deref(),
            Some("3 new workouts archived")
        );
        assert_eq!(
            text(ImportJob::DailyArchive, counts(1, 0, 0)).as_deref(),
            Some("1 day archived")
        );
        for job in ImportJob::ALL {
            assert_eq!(counts_text(job, ImportCounts::default()), None);
            assert!(!no_counts_text(job).is_empty());
        }
    }

    #[test]
    fn progress_reads_in_days_or_as_a_walk() {
        let done_through = NaiveDate::from_ymd_opt(2026, 3, 11).expect("date");
        let days = |left| progress_text(ImportProgress::Days { done_through, left });
        assert_eq!(days(245), "Up to 2026-03-11 · 245 days left");
        assert_eq!(days(1), "Up to 2026-03-11 · 1 day left");
        assert_eq!(days(0), "Complete up to 2026-03-11");
        assert_eq!(
            progress_text(ImportProgress::Workouts { complete: false }),
            "Walking the workout history"
        );
        assert_eq!(
            progress_text(ImportProgress::Workouts { complete: true }),
            "Workout history complete"
        );
    }
}
