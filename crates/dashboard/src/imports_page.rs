use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;
use domain::{ImportEta, Timestamp};
use use_cases::{ImportLine, ImportState, ImportSummary, MAX_RETRIES, job_name, no_counts_text};

use crate::page::{self, Page};
use crate::paris_time::{paris_day, paris_hour_minute, paris_time};

/// What the Imports page shows.
pub enum Imports<'a> {
    /// There is no `[health_collector]` section, so nothing is fetched.
    NotSetUp,
    /// The health collector did not give a usable report.
    Unavailable,
    Report(&'a ImportSummary),
}

struct Card {
    name: &'static str,
    badge_class: &'static str,
    badge: &'static str,
    status: String,
    /// The planned retry, shown right under the status in the primary colour.
    pending: Option<String>,
    details: Vec<String>,
}

#[derive(Template)]
#[template(path = "imports.html")]
struct ImportsPage {
    cards: Option<Vec<Card>>,
    not_set_up: bool,
    updated: String,
    page: Page,
}

/// `Unavailable` renders with status 502, the others with 200.
pub fn render(imports: Imports<'_>, now: Timestamp) -> Response {
    let status = match imports {
        Imports::Unavailable => StatusCode::BAD_GATEWAY,
        Imports::NotSetUp | Imports::Report(_) => StatusCode::OK,
    };
    let page = ImportsPage {
        cards: match imports {
            Imports::Report(summary) => Some(
                summary
                    .lines
                    .iter()
                    .map(|line| card(line, summary.next_pull, now))
                    .collect(),
            ),
            _ => None,
        },
        not_set_up: matches!(imports, Imports::NotSetUp),
        updated: paris_hour_minute(now),
        page: Page::Imports,
    };
    page::respond(&page, status)
}

fn card(line: &ImportLine, next_pull: Option<Timestamp>, now: Timestamp) -> Card {
    let when = |at: Option<Timestamp>| at.map_or_else(|| "—".to_owned(), paris_time);
    let counts = line
        .counts
        .as_deref()
        .unwrap_or_else(|| no_counts_text(line.job));
    let (badge, status) = match line.state {
        ImportState::NotRun => (
            "Not run yet",
            "Not run since the health collector started.".to_owned(),
        ),
        ImportState::Running => ("Running", format!("Running since {}.", when(line.started))),
        ImportState::Worked | ImportState::Partial => (
            if line.state == ImportState::Partial {
                "Partial"
            } else {
                "Up to date"
            },
            format!("Worked at {}: {counts}.", when(line.ended)),
        ),
        ImportState::CatchingUp => (
            "Catching up",
            format!("Paused at {}: {counts}.", when(line.ended)),
        ),
        ImportState::Failed | ImportState::Retrying => (
            if line.state == ImportState::Failed {
                "Failed"
            } else {
                "Retrying"
            },
            format!(
                "{} at {}: {}",
                if line.state == ImportState::Failed {
                    "Failed"
                } else {
                    "Last try failed"
                },
                when(line.ended),
                line.failure.unwrap_or("")
            ),
        ),
    };
    let badge_class = match line.state {
        ImportState::Failed => "badge--alert",
        ImportState::Retrying => "badge--warn",
        _ => "badge--neutral",
    };
    let mut details: Vec<String> = Vec::new();
    if line.state == ImportState::Failed {
        details.push(next_try(line.retry_at, now));
    }
    let retry = retry_text(line);
    let pending = if line.state == ImportState::Retrying {
        retry
    } else {
        details.extend(retry);
        None
    };
    details.extend(line.progress.clone());
    details.extend(line.eta.map(eta_text));
    match line.state {
        ImportState::Failed | ImportState::Retrying => {
            if let Some(counts) = &line.counts {
                details.push(format!("{counts} before it stopped."));
            }
        }
        ImportState::CatchingUp => {
            if let Some(next) = next_pull {
                details.push(format!(
                    "Continues after the next daily pull, {}.",
                    paris_time(next)
                ));
            }
        }
        _ => {}
    }
    Card {
        name: job_name(line.job),
        badge_class,
        badge,
        status,
        pending,
        details,
    }
}

/// What the retries of this window did, or are about to do.
fn retry_text(line: &ImportLine) -> Option<String> {
    let retries = line.retries;
    if line.state == ImportState::Retrying {
        let at = line.next_retry?;
        return Some(format!(
            "Retrying at {} (retry {} of {MAX_RETRIES}).",
            paris_hour_minute(at),
            retries + 1
        ));
    }
    if retries == 0 {
        return None;
    }
    Some(match line.state {
        ImportState::Running => format!("Retry {retries} of {MAX_RETRIES}."),
        ImportState::Worked => format!("Worked on retry {retries}."),
        ImportState::Partial => format!("Partly worked on retry {retries}."),
        ImportState::CatchingUp => format!("Catching up after retry {retries}."),
        ImportState::Failed if retries == 1 => "The retry failed.".to_owned(),
        ImportState::Failed => format!("{retries} retries failed."),
        ImportState::NotRun | ImportState::Retrying => return None,
    })
}

fn next_try(retry_at: Option<Timestamp>, now: Timestamp) -> String {
    match retry_at.filter(|retry| *retry > now) {
        Some(retry) => format!(
            "Tries again at {} ({}).",
            paris_hour_minute(retry),
            paris_day(retry)
        ),
        None => "Will try again at the next daily pull.".to_owned(),
    }
}

fn eta_text(eta: ImportEta) -> String {
    match eta {
        ImportEta::At { at } => format!("Should finish around {}", paris_time(at)),
        ImportEta::ImportDays { days: 1 } => "About 1 more import day".to_owned(),
        ImportEta::ImportDays { days } => format!("About {days} more import days"),
        ImportEta::WaitsFor { job } => format!("Waits for {}", job_name(job)),
        ImportEta::Unknown => "Finish time unknown".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportEta, ImportFailure,
        ImportJob, ImportProgress, ImportStatus, Timestamp,
    };
    use use_cases::{ImportSummary, build_import_summary};

    use crate::page::{Page, assert_shell, at, read};

    use super::{Imports, render};

    fn now() -> Timestamp {
        at("2026-01-15T12:42:00Z")
    }

    fn attempt(outcome: Option<AttemptOutcome>, counts: ImportCounts) -> ImportAttempt {
        ImportAttempt {
            started: at("2026-01-15T09:00:00Z"),
            ended: outcome.map(|outcome| AttemptEnd {
                at: at("2026-01-15T09:30:00Z"),
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

    fn summary(records: &[(ImportJob, ImportAttempt)], retry_at: &str) -> ImportSummary {
        let mut status = ImportStatus {
            next_pull: Some(at("2026-01-16T09:00:00Z")),
            ..ImportStatus::default()
        };
        for (job, attempt) in records {
            status.record(*job, *attempt, at(retry_at), None);
        }
        build_import_summary(&status)
    }

    async fn page(summary: &ImportSummary) -> (StatusCode, String) {
        read(render(Imports::Report(summary), now())).await
    }

    #[tokio::test]
    async fn the_shell_marks_the_imports_page_and_refreshes_every_minute() {
        let (status, html) = page(&summary(&[], "2026-01-16T09:00:00Z")).await;
        assert_eq!(status, StatusCode::OK);
        assert_shell(&html, Page::Imports);
        assert!(html.contains(r#"data-refresh="60""#));
        assert!(html.contains("Updated 13:42"));
        assert!(html.contains("Garmin jobs"));
        assert_eq!(html.matches("Not run yet").count(), 5);
        assert!(html.contains("Not run since the health collector started."));
    }

    #[tokio::test]
    async fn every_state_has_its_badge_and_first_line() {
        let done = attempt(Some(AttemptOutcome::Done), stored(2));
        let partial = attempt(
            Some(AttemptOutcome::Done),
            ImportCounts {
                stored: 1,
                failed: 1,
                ..ImportCounts::default()
            },
        );
        let paused = attempt(Some(AttemptOutcome::Paused), stored(9));
        let running = attempt(None, ImportCounts::default());
        let records = [
            (ImportJob::DailyPull, done),
            (ImportJob::MissedDays, partial),
            (ImportJob::HealthHistory, paused),
            (ImportJob::Workouts, running),
        ];
        let (_, html) = page(&summary(&records, "2026-01-16T09:00:00Z")).await;
        for text in [
            "Up to date",
            "Worked at 2026-01-15 10:30: 2 days pulled.",
            "Partial",
            "Worked at 2026-01-15 10:30: 1 stored, 1 skipped.",
            "Catching up",
            "Paused at 2026-01-15 10:30: 9 stored.",
            "Continues after the next daily pull, 2026-01-16 10:00.",
            "Running",
            "Running since 2026-01-15 10:00.",
            "Not run yet",
        ] {
            assert!(html.contains(text), "{text}");
        }
    }

    #[tokio::test]
    async fn a_failure_shows_the_reason_and_the_next_try_while_it_is_ahead() {
        let failed = attempt(
            Some(AttemptOutcome::Failed(ImportFailure::GarminDown)),
            stored(3),
        );
        let records = [(ImportJob::Workouts, failed)];
        let (_, html) = page(&summary(&records, "2026-01-16T09:00:00Z")).await;
        assert!(html.contains("badge--alert"));
        assert!(html.contains("Failed at 2026-01-15 10:30: Garmin had a server error."));
        assert!(html.contains("Tries again at 10:00 (2026-01-16)."));
        assert!(html.contains("3 new workouts archived before it stopped."));
        assert!(!html.contains("Will try again"));
    }

    #[tokio::test]
    async fn a_failed_card_shows_the_next_try_before_progress_and_finish() {
        let failed = attempt(
            Some(AttemptOutcome::Failed(ImportFailure::GarminDown)),
            ImportCounts::default(),
        );
        let mut status = ImportStatus {
            next_pull: Some(at("2026-01-16T09:00:00Z")),
            ..ImportStatus::default()
        };
        status.record(
            ImportJob::Workouts,
            failed,
            at("2026-01-16T09:00:00Z"),
            None,
        );
        let job = status.jobs.entry(ImportJob::Workouts).or_default();
        job.progress = Some(ImportProgress::Workouts { complete: false });
        job.eta = Some(ImportEta::Unknown);
        let (_, html) = page(&build_import_summary(&status)).await;
        let next = html.find("Tries again at").expect("next try");
        let progress = html.find("Walking the workout history").expect("progress");
        let finish = html.find("Finish time unknown").expect("finish");
        assert!(next < progress && progress < finish);
    }

    fn try_at(hour: u32, outcome: Option<AttemptOutcome>, counts: ImportCounts) -> ImportAttempt {
        ImportAttempt {
            started: at(&format!("2026-01-15T{hour:02}:00:00Z")),
            ended: outcome.map(|outcome| AttemptEnd {
                at: at(&format!("2026-01-15T{hour:02}:01:00Z")),
                outcome,
                counts,
            }),
        }
    }

    fn down() -> AttemptOutcome {
        AttemptOutcome::Failed(ImportFailure::GarminDown)
    }

    fn retry_at(hour: u32) -> Option<Timestamp> {
        Some(at(&format!("2026-01-15T{hour:02}:30:00Z")))
    }

    /// The workouts job after `tries`, oldest first, each with the retry
    /// planned when it ended.
    async fn workouts_page(tries: &[(ImportAttempt, Option<Timestamp>)]) -> String {
        let mut status = ImportStatus {
            next_pull: Some(at("2026-01-16T09:00:00Z")),
            ..ImportStatus::default()
        };
        for (attempt, retry) in tries {
            status.record(
                ImportJob::Workouts,
                *attempt,
                at("2026-01-16T09:00:00Z"),
                *retry,
            );
        }
        page(&build_import_summary(&status)).await.1
    }

    #[tokio::test]
    async fn a_pending_retry_is_a_warning_with_its_time_before_the_other_details() {
        let first = try_at(10, Some(down()), stored(3));
        let mut status = ImportStatus {
            next_pull: Some(at("2026-01-16T09:00:00Z")),
            ..ImportStatus::default()
        };
        status.record(
            ImportJob::Workouts,
            first,
            at("2026-01-16T09:00:00Z"),
            retry_at(12),
        );
        let job = status.jobs.entry(ImportJob::Workouts).or_default();
        job.progress = Some(ImportProgress::Workouts { complete: false });
        let (_, html) = page(&build_import_summary(&status)).await;
        assert!(html.contains(r#"badge--warn">Retrying"#));
        assert!(html.contains("Last try failed at 2026-01-15 11:01: Garmin had a server error."));
        assert!(html.contains(r#"secondary secondary--now">Retrying at 13:30"#));
        assert!(html.contains("Retrying at 13:30 (retry 1 of 3)."));
        assert!(!html.contains("Tries again at"));
        let retry = html.find("Retrying at").expect("retry line");
        let progress = html.find("Walking the workout history").expect("progress");
        let counts = html.find("before it stopped").expect("counts");
        assert!(retry < progress && progress < counts);
    }

    #[tokio::test]
    async fn the_second_pending_retry_counts_the_attempts_so_far() {
        let html = workouts_page(&[
            (
                try_at(10, Some(down()), ImportCounts::default()),
                retry_at(10),
            ),
            (
                try_at(11, Some(down()), ImportCounts::default()),
                retry_at(12),
            ),
        ])
        .await;
        assert!(html.contains("Retrying at 13:30 (retry 2 of 3)."));
    }

    #[tokio::test]
    async fn a_running_retry_says_which_retry_it_is() {
        let html = workouts_page(&[
            (
                try_at(10, Some(down()), ImportCounts::default()),
                retry_at(10),
            ),
            (try_at(11, None, ImportCounts::default()), None),
        ])
        .await;
        assert!(html.contains("Running since 2026-01-15 12:00."));
        assert!(html.contains("Retry 1 of 3."));
        assert!(!html.contains("Retrying"));
    }

    #[tokio::test]
    async fn a_retry_that_ended_says_how_it_ended() {
        let partial = ImportCounts {
            stored: 1,
            failed: 1,
            ..ImportCounts::default()
        };
        for (outcome, counts, text) in [
            (AttemptOutcome::Done, stored(2), "Worked on retry 1."),
            (AttemptOutcome::Done, partial, "Partly worked on retry 1."),
            (
                AttemptOutcome::Paused,
                stored(2),
                "Catching up after retry 1.",
            ),
            (
                AttemptOutcome::Capped,
                stored(2),
                "Catching up after retry 1.",
            ),
        ] {
            let html = workouts_page(&[
                (
                    try_at(10, Some(down()), ImportCounts::default()),
                    retry_at(10),
                ),
                (try_at(11, Some(outcome), counts), None),
            ])
            .await;
            assert!(html.contains(text), "{text}");
            assert!(!html.contains("Retrying"));
        }
    }

    #[tokio::test]
    async fn a_give_up_after_retries_says_they_failed_after_the_next_try() {
        let tries: Vec<_> = (10..14)
            .map(|hour| {
                let retry = (hour < 13).then(|| at(&format!("2026-01-15T{hour:02}:30:00Z")));
                (try_at(hour, Some(down()), ImportCounts::default()), retry)
            })
            .collect();
        let html = workouts_page(&tries).await;
        assert!(html.contains("badge--alert"));
        assert!(html.contains("3 retries failed."));
        let next = html.find("Tries again at").expect("next try");
        assert!(next < html.find("3 retries failed.").expect("retry line"));
        let html = workouts_page(&tries[2..]).await;
        assert!(!html.contains("retries failed"));
        let html = workouts_page(&[
            (
                try_at(10, Some(down()), ImportCounts::default()),
                retry_at(10),
            ),
            (try_at(11, Some(down()), ImportCounts::default()), None),
        ])
        .await;
        assert!(html.contains("The retry failed."));
    }

    #[tokio::test]
    async fn a_job_without_retries_has_no_retry_line() {
        let done = attempt(Some(AttemptOutcome::Done), stored(2));
        let failed = attempt(Some(down()), ImportCounts::default());
        let records = [(ImportJob::MissedDays, done), (ImportJob::Workouts, failed)];
        let (_, html) = page(&summary(&records, "2026-01-16T09:00:00Z")).await;
        for text in ["Retry", "retry", "Retrying"] {
            assert!(!html.contains(text), "{text}");
        }
    }

    #[tokio::test]
    async fn a_retry_time_that_has_passed_falls_back_to_the_next_daily_pull() {
        let failed = attempt(
            Some(AttemptOutcome::Failed(ImportFailure::ProgressUnreadable)),
            ImportCounts::default(),
        );
        let records = [(ImportJob::DailyArchive, failed)];
        let (_, html) = page(&summary(&records, "2026-01-15T09:00:00Z")).await;
        assert!(html.contains("Will try again at the next daily pull."));
        assert!(html.contains("needs attention; fix or delete it on the Pi."));
        assert!(!html.contains("Tries again at"));
    }

    #[tokio::test]
    async fn progress_and_every_finish_estimate_have_their_copy() {
        let mut status = ImportStatus {
            next_pull: Some(at("2026-01-16T09:00:00Z")),
            ..ImportStatus::default()
        };
        let etas = [
            (
                ImportJob::MissedDays,
                ImportEta::At {
                    at: at("2026-01-15T14:47:00Z"),
                },
            ),
            (ImportJob::HealthHistory, ImportEta::ImportDays { days: 1 }),
            (ImportJob::Workouts, ImportEta::Unknown),
            (
                ImportJob::DailyArchive,
                ImportEta::WaitsFor {
                    job: ImportJob::HealthHistory,
                },
            ),
        ];
        for (job, eta) in etas {
            status.jobs.entry(job).or_default().eta = Some(eta);
        }
        status
            .jobs
            .entry(ImportJob::HealthHistory)
            .or_default()
            .progress = Some(ImportProgress::Workouts { complete: false });
        let (_, html) = page(&build_import_summary(&status)).await;
        for text in [
            "Should finish around 2026-01-15 15:47",
            "About 1 more import day",
            "Finish time unknown",
            "Waits for One-year health import",
            "Walking the workout history",
        ] {
            assert!(html.contains(text), "{text}");
        }
        status.jobs.entry(ImportJob::HealthHistory).or_default().eta =
            Some(ImportEta::ImportDays { days: 3 });
        let (_, html) = page(&build_import_summary(&status)).await;
        assert!(html.contains("About 3 more import days"));
    }

    #[tokio::test]
    async fn an_unavailable_collector_is_a_bad_gateway_with_a_notice() {
        let (status, html) = read(render(Imports::Unavailable, now())).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Import status unavailable."));
        assert!(html.contains(r#"role="status""#));
        assert_shell(&html, Page::Imports);
        assert!(!html.contains("Not run yet"));
    }

    #[tokio::test]
    async fn without_a_health_collector_the_page_says_garmin_is_not_set_up() {
        let (status, html) = read(render(Imports::NotSetUp, now())).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Garmin is not set up."));
        assert!(html.contains(r#"role="status""#));
        assert!(html.contains("badge--neutral\">Not set up"));
        assert!(!html.contains("Import status unavailable"));
    }

    #[tokio::test]
    async fn report_text_is_escaped_and_the_page_has_no_links_out() {
        let mut summary = summary(&[], "2026-01-16T09:00:00Z");
        summary.lines[0].progress = Some("<script>sentinel</script>".to_owned());
        let (_, html) = page(&summary).await;
        assert!(!html.contains("<script>sentinel"));
        assert!(html.contains("&#60;script&#62;sentinel"));
        assert!(!html.contains("http://") && !html.contains("https://"));
    }
}
