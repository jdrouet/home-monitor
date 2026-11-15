use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;
use domain::{AlertKind, Timestamp};
use use_cases::AlertSummary;

use crate::page::{self, Page};
use crate::paris_time::paris_time;

#[derive(Template)]
#[template(path = "alerts.html")]
struct AlertsPage<'a> {
    summary: Option<&'a AlertSummary>,
    page: Page,
}

impl AlertsPage<'_> {
    fn time(&self, at: &Timestamp) -> String {
        paris_time(*at)
    }
}

/// `None` renders the page for an alerter that cannot be reached.
pub fn render(summary: Option<&AlertSummary>) -> Response {
    let page = AlertsPage {
        summary,
        page: Page::Alerts,
    };
    page::respond(&page, StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use chrono::DateTime;
    use domain::{AlertKind, Timestamp};
    use use_cases::{ActiveAlertLine, AlertSummary, ResolvedAlertLine};

    use crate::page::{Page, assert_shell};

    use super::render;

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    async fn page(summary: Option<&AlertSummary>) -> String {
        let bytes = to_bytes(render(summary).into_body(), usize::MAX)
            .await
            .expect("body reads");
        String::from_utf8(bytes.to_vec()).expect("utf-8 body")
    }

    fn summary() -> AlertSummary {
        AlertSummary {
            active: vec![
                ActiveAlertLine {
                    subject: "bedroom".to_owned(),
                    kind: AlertKind::TooHot,
                    celsius: Some(21.3),
                    since: at("2026-01-15T12:30:00Z"),
                },
                ActiveAlertLine {
                    subject: "office".to_owned(),
                    kind: AlertKind::Offline,
                    celsius: None,
                    since: at("2026-01-15T10:00:00Z"),
                },
                ActiveAlertLine {
                    subject: "Garmin".to_owned(),
                    kind: AlertKind::GarminLogin,
                    celsius: None,
                    since: at("2026-01-15T09:00:00Z"),
                },
            ],
            resolved: vec![
                ResolvedAlertLine {
                    subject: "living room".to_owned(),
                    kind: AlertKind::TooCold,
                    started: at("2026-07-15T08:00:00Z"),
                    ended: at("2026-07-15T23:30:00Z"),
                },
                ResolvedAlertLine {
                    subject: "Garmin".to_owned(),
                    kind: AlertKind::GarminLogin,
                    started: at("2026-03-01T08:00:00Z"),
                    ended: at("2026-03-02T08:00:00Z"),
                },
            ],
        }
    }

    fn empty() -> AlertSummary {
        AlertSummary {
            active: Vec::new(),
            resolved: Vec::new(),
        }
    }

    #[tokio::test]
    async fn active_alerts_show_room_kind_temperature_and_start() {
        let html = page(Some(&summary())).await;
        assert!(html.contains("bedroom"));
        assert!(html.contains("too hot"));
        assert!(html.contains("21.3 °C at last notification"));
        assert!(html.contains("since 2026-01-15 13:30"));
        assert!(html.contains("office"));
        assert!(html.contains("offline"));
        assert!(html.contains("Garmin: login expired, since 2026-01-15 10:00"));
        assert!(!html.contains("No active alerts."));
    }

    #[tokio::test]
    async fn resolved_alerts_show_start_and_end() {
        let html = page(Some(&summary())).await;
        assert!(html.contains("Resolved in the last 30 days"));
        assert!(html.contains("living room"));
        assert!(html.contains("too cold"));
        assert!(html.contains("2026-07-15 10:00"));
        assert!(html.contains("2026-07-16 01:30"));
        assert!(html.contains("Garmin: login expired, from 2026-03-01 09:00 to 2026-03-02 09:00"));
        assert!(!html.contains("No alerts resolved"));
    }

    #[tokio::test]
    async fn empty_sections_say_so() {
        let html = page(Some(&empty())).await;
        assert!(html.contains("No active alerts."));
        assert!(html.contains("No alerts resolved in the last 30 days."));
        assert!(!html.contains("Alerts unavailable"));
    }

    #[tokio::test]
    async fn without_a_summary_the_page_says_alerts_are_unavailable() {
        let html = page(None).await;
        assert!(html.contains("Alerts unavailable"));
        assert_shell(&html, Page::Alerts);
        assert!(!html.contains("No active alerts."));
    }

    #[tokio::test]
    async fn room_names_are_escaped() {
        let mut summary = empty();
        summary.active.push(ActiveAlertLine {
            subject: "<script>alert(1)</script>".to_owned(),
            kind: AlertKind::Offline,
            celsius: None,
            since: at("2026-01-15T10:00:00Z"),
        });
        let html = page(Some(&summary)).await;
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&#60;script&#62;"));
    }
}
