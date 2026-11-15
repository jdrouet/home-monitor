use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use tracing::error;
use use_cases::HealthRow;

const MISSING: &str = "—";

#[derive(Template)]
#[template(path = "health.html")]
struct HealthPage<'a> {
    rows: &'a [HealthRow],
}

impl HealthPage<'_> {
    /// `7 h 32 min`, rounded to the minute.
    fn duration(&self, seconds: &Option<f64>) -> String {
        seconds.map_or_else(
            || MISSING.to_owned(),
            |seconds| {
                let minutes = (seconds / 60.0).round() as u64;
                format!("{} h {} min", minutes / 60, minutes % 60)
            },
        )
    }

    fn number(&self, value: &Option<f64>) -> String {
        value.map_or_else(|| MISSING.to_owned(), |value| format!("{value:.0}"))
    }
}

pub fn render(rows: &[HealthRow]) -> Response {
    match (HealthPage { rows }).render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            error!(%error, "cannot render health page");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use chrono::NaiveDate;
    use use_cases::HealthRow;

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

    async fn page(rows: &[HealthRow]) -> String {
        let bytes = to_bytes(render(rows).into_body(), usize::MAX)
            .await
            .expect("body reads");
        String::from_utf8(bytes.to_vec()).expect("utf-8 body")
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
        assert!(html.contains("<td>2026-07-14</td>"));
        assert!(html.contains("<td>7 h 32 min</td>"));
        assert!(html.contains("<td>81</td>"));
        assert!(html.contains("<td>52</td>"));
        assert!(html.contains("<td>8431</td>"));
        assert!(html.contains("<td>96</td>"));
        assert!(html.contains("<td>14</td>"));
        assert!(!html.contains("—"));
    }

    #[tokio::test]
    async fn durations_round_to_the_minute() {
        let mut short = row(14);
        short.sleep_seconds = Some(3_599.0);
        short.sleep_score = Some(80.6);
        let html = page(&[short]).await;
        assert!(html.contains("<td>1 h 0 min</td>"));
        assert!(html.contains("<td>81</td>"));
    }

    #[tokio::test]
    async fn missing_figures_are_dashes() {
        let html = page(&[row(14), row(13)]).await;
        assert_eq!(html.matches("<td>—</td>").count(), 12);
        assert!(html.contains("<td>2026-07-13</td>"));
    }

    #[tokio::test]
    async fn the_page_links_to_the_other_pages() {
        let html = page(&[]).await;
        assert!(html.contains(r#"<a href="/">Overview</a>"#));
        assert!(html.contains(r#"<a href="/history">History</a>"#));
        assert!(html.contains(r#"<a href="/alerts">Alerts</a>"#));
        assert!(html.contains("Body Battery max"));
    }
}
