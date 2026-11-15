use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use domain::{Metric, Thermometer};
use tracing::error;
use use_cases::{HISTORY_METRICS, HistoryChart, HistorySelection, Period};

#[derive(Template)]
#[template(path = "history.html")]
struct HistoryPage<'a> {
    selection: &'a HistorySelection,
    thermometers: &'a [Thermometer],
    chart: &'a HistoryChart,
    metrics: [Metric; 2],
    periods: [Period; 4],
}

pub fn render(
    selection: &HistorySelection,
    thermometers: &[Thermometer],
    chart: &HistoryChart,
) -> Response {
    let page = HistoryPage {
        selection,
        thermometers,
        chart,
        metrics: HISTORY_METRICS,
        periods: Period::ALL,
    };
    match page.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            error!(%error, "cannot render history page");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use domain::{MacAddress, Metric, RoomName, Thermometer};
    use use_cases::{HistoryChart, HistorySelection, Period};

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

    async fn page(rooms: &[&str], selected: &HistorySelection, chart: &HistoryChart) -> String {
        let bytes = to_bytes(
            render(selected, &thermometers(rooms), chart).into_body(),
            usize::MAX,
        )
        .await
        .expect("body reads");
        String::from_utf8(bytes.to_vec()).expect("utf-8 body")
    }

    #[tokio::test]
    async fn the_form_shows_rooms_outside_and_the_selection() {
        let selected = selection(&["office"], true, Period::Week);
        let html = page(&["bedroom", "office"], &selected, &chart(&["office"])).await;
        assert!(html.contains(r#"value="bedroom">"#));
        assert!(html.contains(r#"value="office" checked>"#));
        assert!(html.contains(r#"name="outside" value="on" checked>"#));
        assert!(html.contains(r#"<option value="7d" selected>"#));
        assert!(html.contains(r#"<option value="24h">"#));
        assert!(html.contains(r#"<option value="temperature" selected>"#));
        assert!(html.contains(r#"<option value="humidity">"#));
    }

    #[tokio::test]
    async fn outside_is_unchecked_when_not_selected() {
        let selected = selection(&["office"], false, Period::Day);
        let html = page(&["office"], &selected, &chart(&["office"])).await;
        assert!(html.contains(r#"name="outside" value="on">"#));
    }

    #[tokio::test]
    async fn the_page_links_the_assets_and_the_overview() {
        let selected = selection(&[], false, Period::Day);
        let html = page(&[], &selected, &chart(&[])).await;
        assert!(html.contains(r#"href="/assets/uplot-1.6.32.min.css""#));
        assert!(html.contains(r#"src="/assets/uplot-1.6.32.min.js""#));
        assert!(html.contains(r#"href="/""#));
        assert!(html.contains(r#"<a href="/alerts">Alerts</a>"#));
        assert!(html.contains(r#"<div id="chart">"#));
        assert!(html.contains("const timestamps = [0,900];"));
        assert!(html.contains("const lines = [];"));
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
