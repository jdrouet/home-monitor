use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;
use domain::{Metric, Thermometer};
use use_cases::{HISTORY_METRICS, HistoryChart, HistorySelection, Period};

use crate::page::{self, Page};

#[derive(Template)]
#[template(path = "history.html")]
struct HistoryPage<'a> {
    selection: &'a HistorySelection,
    thermometers: &'a [Thermometer],
    chart: Option<&'a HistoryChart>,
    metrics: [Metric; 2],
    periods: [Period; 4],
    page: Page,
}

impl HistoryPage<'_> {
    fn has_readings(&self) -> bool {
        self.chart
            .is_some_and(|chart| chart.lines.iter().flatten().any(Option::is_some))
    }
}

/// `None` renders the form with a notice for a store that cannot be reached,
/// with status 502.
pub fn render(
    selection: &HistorySelection,
    thermometers: &[Thermometer],
    chart: Option<&HistoryChart>,
) -> Response {
    let page = HistoryPage {
        selection,
        thermometers,
        chart,
        metrics: HISTORY_METRICS,
        periods: Period::ALL,
        page: Page::History,
    };
    let status = chart.map_or(StatusCode::BAD_GATEWAY, |_| StatusCode::OK);
    page::respond(&page, status)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use domain::{MacAddress, Metric, RoomName, Thermometer};
    use use_cases::{HistoryChart, HistorySelection, Period};

    use crate::page::{Page, assert_shell, read};

    use super::render;

    fn thermometers(rooms: &[&str]) -> Vec<Thermometer> {
        rooms
            .iter()
            .zip(1..)
            .map(|(room, last)| Thermometer {
                mac: MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, last]),
                room: RoomName::try_from((*room).to_owned()).expect("valid room"),
                temperature_range: None,
            })
            .collect()
    }

    fn selection(rooms: &[&str], include_outside: bool, period: Period) -> HistorySelection {
        HistorySelection {
            metric: Metric::Temperature,
            period,
            rooms: rooms
                .iter()
                .map(|room| RoomName::try_from((*room).to_owned()).expect("valid room"))
                .collect(),
            include_outside,
        }
    }

    fn chart(labels: &[&str]) -> HistoryChart {
        HistoryChart {
            labels: labels.iter().map(|label| (*label).to_owned()).collect(),
            timestamps: vec![0, 900],
            lines: labels.iter().map(|_| vec![Some(1.5), None]).collect(),
        }
    }

    async fn response(
        rooms: &[&str],
        selected: &HistorySelection,
        chart: Option<&HistoryChart>,
    ) -> (StatusCode, String) {
        read(render(selected, &thermometers(rooms), chart)).await
    }

    async fn page(rooms: &[&str], selected: &HistorySelection, chart: &HistoryChart) -> String {
        response(rooms, selected, Some(chart)).await.1
    }

    #[tokio::test]
    async fn the_form_shows_rooms_outside_and_the_selection() {
        let selected = selection(&["office"], true, Period::Week);
        let html = page(&["bedroom", "office"], &selected, &chart(&["office"])).await;
        assert!(html.contains(r#"value="bedroom">"#));
        assert!(html.contains(r#"value="office" checked>"#));
        assert!(html.contains(r#"name="outside" value="on" checked>"#));
        assert!(html.contains(r#"value="7d" checked>"#));
        assert!(html.contains(r#"value="24h">"#));
        assert!(html.contains(r#"value="temperature" checked>"#));
        assert!(html.contains(r#"value="humidity">"#));
        assert!(html.contains("Temperature, last 7 days"));
        assert!(html.contains("Update chart"));
    }

    #[tokio::test]
    async fn outside_is_unchecked_when_not_selected() {
        let selected = selection(&["office"], false, Period::Day);
        let html = page(&["office"], &selected, &chart(&["office"])).await;
        assert!(html.contains(r#"name="outside" value="on">"#));
    }

    #[tokio::test]
    async fn the_page_links_the_assets_and_the_overview() {
        let selected = selection(&["office"], false, Period::Day);
        let html = page(&["office"], &selected, &chart(&["office"])).await;
        assert!(html.contains(r#"href="/assets/uplot-1.6.32.min.css""#));
        assert!(html.contains(r#"src="/assets/uplot-1.6.32.min.js""#));
        assert_shell(&html, Page::History);
        assert!(!html.contains("location.reload"));
        assert!(!html.contains("data-refresh"));
        assert!(html.contains(r#"<div id="chart">"#));
        assert!(html.contains("const timestamps = [0,900];"));
        assert!(html.contains("const lines = [[1.5,null]];"));
        assert!(html.contains(r#"token("--chart-" + "#));
    }

    #[tokio::test]
    async fn no_readings_shows_the_empty_notice() {
        let selected = selection(&["office"], false, Period::Day);
        let empty = HistoryChart {
            labels: vec!["office".to_owned()],
            timestamps: vec![0, 900],
            lines: vec![vec![None, None]],
        };
        let (status, html) = response(&["office"], &selected, Some(&empty)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("No readings in this period."));
        assert!(!html.contains(r#"<div id="chart">"#));
        assert!(!html.contains("const timestamps"));
    }

    #[tokio::test]
    async fn an_unavailable_store_keeps_the_form_and_shows_the_notice() {
        let selected = selection(&["office"], false, Period::Day);
        let (status, html) = response(&["office"], &selected, None).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("History unavailable"));
        assert!(html.contains(r#"value="office" checked>"#));
        assert!(!html.contains("No readings in this period."));
        assert_shell(&html, Page::History);
    }

    #[tokio::test]
    async fn gaps_are_null_in_the_chart_data() {
        let selected = selection(&["office"], false, Period::Day);
        let html = page(&["office"], &selected, &chart(&["office"])).await;
        assert!(html.contains("const lines = [[1.5,null]];"));
    }

    #[tokio::test]
    async fn a_room_name_cannot_break_out_of_the_script() {
        let name = "</script><script>alert(1)</script>";
        let selected = selection(&[name], false, Period::Day);
        let html = page(&[name], &selected, &chart(&[name])).await;
        assert!(!html.contains("<script>alert(1)"));
        let data = html
            .split("const labels = ")
            .nth(1)
            .expect("labels constant")
            .split(';')
            .next()
            .expect("statement");
        assert!(!data.contains("</script"));
        assert!(html.contains("&#60;/script&#62;&#60;script&#62;"));
    }
}
