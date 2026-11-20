use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;
use domain::{ImportEta, Timestamp};
use use_cases::{ImportLine, ImportState, ImportSummary, job_name};

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
    let (badge_class, badge, status) = match line.state {
        ImportState::NotRun => (
            "badge--neutral",
            "Not run yet",
            "Not run since the health collector started.".to_owned(),
        ),
        ImportState::Running => (
            "badge--neutral",
            "Running",
            format!("Running since {}.", when(line.started)),
        ),
        ImportState::Worked => (
            "badge--neutral",
            "Up to date",
            format!("Worked at {}: {}.", when(line.ended), line.counts),
        ),
        ImportState::Partial => (
            "badge--neutral",
            "Partial",
            format!("Worked at {}: {}.", when(line.ended), line.counts),
        ),
        ImportState::CatchingUp => (
            "badge--neutral",
            "Catching up",
            format!("Paused at {}: {}.", when(line.ended), line.counts),
        ),
        ImportState::Failed => (
            "badge--alert",
            "Failed",
            format!(
                "Failed at {}: {}",
                when(line.ended),
                line.failure.unwrap_or("")
            ),
        ),
    };
    let mut details: Vec<String> = Vec::new();
    if line.state == ImportState::Failed {
        details.push(next_try(line.retry_at, now));
    }
    details.extend(line.progress.clone());
    details.extend(line.eta.map(eta_text));
    match line.state {
        ImportState::Failed => {
            if line.counted {
                details.push(format!("{} before it stopped.", line.counts));
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
        name: line.name,
        badge_class,
        badge,
        status,
        details,
    }
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
            status.record(*job, *attempt, at(retry_at));
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
        status.record(ImportJob::Workouts, failed, at("2026-01-16T09:00:00Z"));
        let job = status.jobs.entry(ImportJob::Workouts).or_default();
        job.progress = Some(ImportProgress::Workouts { complete: false });
        job.eta = Some(ImportEta::Unknown);
        let (_, html) = page(&build_import_summary(&status)).await;
        let next = html.find("Tries again at").expect("next try");
        let progress = html.find("Walking the workout history").expect("progress");
        let finish = html.find("Finish time unknown").expect("finish");
        assert!(next < progress && progress < finish);
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
