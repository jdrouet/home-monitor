use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;
use use_cases::HealthRow;

use crate::page::{self, Page};

#[derive(Template)]
#[template(path = "health.html")]
struct HealthPage<'a> {
    rows: Option<&'a [HealthRow]>,
    page: Page,
}

impl HealthPage<'_> {
    /// `7 h 32 min`, rounded to the minute.
    fn duration(&self, seconds: &Option<f64>) -> Option<String> {
        seconds.map(|seconds| {
            let minutes = (seconds / 60.0).round() as u64;
            format!("{} h {} min", minutes / 60, minutes % 60)
        })
    }

    fn number(&self, value: &Option<f64>) -> Option<String> {
        value.map(|value| format!("{value:.0}"))
    }

    /// True when at least one row has at least one figure.
    fn has_data(&self) -> bool {
        self.rows.is_some_and(|rows| {
            rows.iter().any(|row| {
                [
                    row.sleep_seconds,
                    row.sleep_score,
                    row.resting_heart_rate,
                    row.steps,
                    row.body_battery_max,
                    row.body_battery_min,
                ]
                .iter()
                .any(Option::is_some)
            })
        })
    }
}

/// `None` renders a notice for a store that cannot be reached, with status 502.
pub fn render(rows: Option<&[HealthRow]>) -> Response {
    let page = HealthPage {
        rows,
        page: Page::Health,
    };
    let status = rows.map_or(StatusCode::BAD_GATEWAY, |_| StatusCode::OK);
    page::respond(&page, status)
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use chrono::NaiveDate;
    use use_cases::HealthRow;

    use crate::page::{Page, assert_shell};

    use super::render;

    fn row(day: u32) -> HealthRow {
        HealthRow {
            date: NaiveDate::from_ymd_opt(2026, 7, day).expect("valid date"),
            sleep_seconds: None,
            sleep_score: None,
            resting_heart_rate: None,
            steps: None,
            body_battery_max: None,
            body_battery_min: None,
        }
    }

    async fn response(rows: Option<&[HealthRow]>) -> (StatusCode, String) {
        let response = render(rows);
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        (
            status,
            String::from_utf8(bytes.to_vec()).expect("utf-8 body"),
        )
    }

    async fn page(rows: &[HealthRow]) -> String {
        response(Some(rows)).await.1
    }

    #[tokio::test]
    async fn a_full_row_shows_every_figure() {
        let mut full = row(14);
        full.sleep_seconds = Some(27_120.0);
        full.sleep_score = Some(81.0);
        full.resting_heart_rate = Some(52.0);
        full.steps = Some(8431.0);
        full.body_battery_max = Some(96.0);
        full.body_battery_min = Some(14.0);
        let html = page(&[full]).await;
        assert!(html.contains(r#"<th scope="row">2026-07-14</th>"#));
        assert!(html.contains(r#"<td class="num">7 h 32 min</td>"#));
        assert!(html.contains(r#"<td class="num">81</td>"#));
        assert!(html.contains(r#"<td class="num">52</td>"#));
        assert!(html.contains(r#"<td class="num">8431</td>"#));
        assert!(html.contains(r#"<td class="num">96</td>"#));
        assert!(html.contains(r#"<td class="num">14</td>"#));
        assert!(!html.contains("no data"));
    }

    #[tokio::test]
    async fn durations_round_to_the_minute() {
        let mut short = row(14);
        short.sleep_seconds = Some(3_599.0);
        short.sleep_score = Some(80.6);
        let html = page(&[short]).await;
        assert!(html.contains(r#"<td class="num">1 h 0 min</td>"#));
        assert!(html.contains(r#"<td class="num">81</td>"#));
    }

    #[tokio::test]
    async fn missing_figures_are_marked_as_no_data() {
        let mut some = row(14);
        some.steps = Some(100.0);
        let html = page(&[some, row(13)]).await;
        assert_eq!(html.matches("no data").count(), 11);
        assert!(html.contains(r#"<th scope="row">2026-07-13</th>"#));
    }

    #[tokio::test]
    async fn the_table_region_is_keyboard_reachable_and_labelled() {
        let mut some = row(14);
        some.steps = Some(100.0);
        let html = page(&[some]).await;
        assert!(html.contains(r#"role="region" tabindex="0" aria-labelledby="health-caption""#));
        assert!(html.contains(r#"<caption id="health-caption""#));
    }

    #[tokio::test]
    async fn rows_without_any_figure_show_the_empty_notice() {
        let html = page(&[]).await;
        assert_shell(&html, Page::Health);
        assert!(html.contains("No health data in the last 30 days"));
        assert!(!html.contains("<table"));
        assert!(html.contains("}, 300 * 1000);"));
    }

    #[tokio::test]
    async fn an_unavailable_store_shows_the_notice() {
        let (status, html) = response(None).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Health data unavailable"));
        assert!(!html.contains("<table"));
        assert_shell(&html, Page::Health);
    }
}
