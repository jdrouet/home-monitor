use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;
use domain::{AlertKind, Timestamp};
use use_cases::{ActiveAlertLine, AlertSummary, Freshness, Overview};

use crate::page::{self, Page};
use crate::paris_time::paris_hour_minute;

/// Rows the overview shows; beyond it the last row links to the alerts page.
const MAX_ALERT_ROWS: usize = 3;

#[derive(Template)]
#[template(path = "overview.html")]
struct OverviewPage<'a> {
    overview: Option<&'a Overview>,
    alerts: Option<&'a AlertSummary>,
    updated: String,
    page: Page,
}

impl OverviewPage<'_> {
    fn active(&self) -> &[ActiveAlertLine] {
        self.alerts.map_or(&[], |alerts| &alerts.active)
    }

    /// All active alerts, or the oldest ones that leave room for the link row.
    fn shown_alerts(&self) -> &[ActiveAlertLine] {
        let active = self.active();
        if active.len() > MAX_ALERT_ROWS {
            &active[..MAX_ALERT_ROWS - 1]
        } else {
            active
        }
    }

    fn more_alerts(&self) -> usize {
        self.active().len() - self.shown_alerts().len()
    }
}

/// `None` for `overview` renders the store notice with status 502; `None`
/// for `alerts` renders the alerter notice and keeps the rooms.
pub fn render(
    overview: Option<&Overview>,
    alerts: Option<&AlertSummary>,
    now: Timestamp,
) -> Response {
    let page = OverviewPage {
        overview,
        alerts,
        updated: paris_hour_minute(now),
        page: Page::Overview,
    };
    let status = overview.map_or(StatusCode::BAD_GATEWAY, |_| StatusCode::OK);
    page::respond(&page, status)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use domain::{AlertKind, MacAddress, RoomName, SourceId};
    use use_cases::{
        ActiveAlertLine, AlertSummary, ClimateReading, Freshness, OutdoorOverview, Overview,
        RoomOverview, SensorOverview,
    };

    use crate::page::{Page, assert_shell, at, read};

    use super::render;

    fn reading(
        temperature: Option<f64>,
        humidity: Option<f64>,
        freshness: Freshness,
    ) -> ClimateReading {
        ClimateReading {
            temperature,
            humidity,
            freshness,
        }
    }

    fn outdoor(freshness: Freshness) -> OutdoorOverview {
        OutdoorOverview {
            reading: reading(Some(4.8), Some(78.0), freshness),
            temperature_min: Some(2.4),
            temperature_max: Some(11.0),
            summary: Some("Overcast"),
        }
    }

    fn room(name: &str, reading: ClimateReading) -> RoomOverview {
        RoomOverview {
            room: RoomName::try_from(name.to_owned()).expect("valid room"),
            reading,
        }
    }

    fn overview(rooms: Vec<RoomOverview>) -> Overview {
        Overview {
            outdoor: outdoor(Freshness::Reporting),
            rooms,
            unknown_sensors: Vec::new(),
        }
    }

    fn active(count: usize) -> AlertSummary {
        AlertSummary {
            active: (1..=count)
                .map(|n| ActiveAlertLine {
                    subject: format!("room{n}"),
                    kind: AlertKind::TooHot,
                    celsius: Some(21.3),
                    since: at("2026-01-15T12:30:00Z"),
                })
                .collect(),
            resolved: Vec::new(),
        }
    }

    async fn response(
        overview: Option<&Overview>,
        alerts: Option<&AlertSummary>,
    ) -> (StatusCode, String) {
        read(render(overview, alerts, at("2026-01-15T12:30:00Z"))).await
    }

    async fn page(overview: &Overview) -> String {
        response(Some(overview), Some(&active(0))).await.1
    }

    #[tokio::test]
    async fn page_shows_rooms_and_unknown_sensors() {
        let mut overview = overview(vec![
            room(
                "bedroom",
                reading(Some(21.27), Some(51.49), Freshness::Reporting),
            ),
            room("office", reading(None, None, Freshness::NotReporting)),
        ]);
        overview.unknown_sensors = vec![SensorOverview {
            source_id: SourceId::from(MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9])),
            reading: reading(Some(18.0), None, Freshness::Reporting),
        }];
        let html = page(&overview).await;
        assert!(html.contains("bedroom"));
        assert!(html.contains(r#"<span class="metric-value">21.3</span>"#));
        assert!(html.contains(r#"<span class="metric-unit">°C</span>"#));
        assert!(html.contains("51.5 % humidity"));
        assert!(html.contains("Not reporting"));
        assert!(html.contains("no reading"));
        assert!(html.contains("Unknown sensors"));
        assert!(html.contains("Not assigned to a room in config.toml."));
        assert!(html.contains("A4:C1:38:00:00:09"));
    }

    #[tokio::test]
    async fn unknown_section_is_hidden_when_empty() {
        let overview = overview(vec![room(
            "bedroom",
            reading(Some(20.0), Some(50.0), Freshness::Reporting),
        )]);
        let html = page(&overview).await;
        assert!(!html.contains("Unknown sensors"));
        assert!(!html.contains("Not reporting"));
        assert!(html.contains(r#"<span class="metric-value">20.0</span>"#));
    }

    #[tokio::test]
    async fn available_weather_shows_the_outside_section() {
        let html = page(&overview(Vec::new())).await;
        assert!(html.contains("<h3>Outside</h3>"));
        assert!(html.contains(r#"<span class="metric-value">4.8</span>"#));
        assert!(html.contains("78.0 % humidity"));
        assert!(html.contains("Overcast"));
        assert!(html.contains("2.4–11.0 °C"));
        assert!(!html.contains("Weather unavailable"));
    }

    #[tokio::test]
    async fn unavailable_weather_keeps_the_rooms() {
        let mut overview = overview(vec![room(
            "bedroom",
            reading(Some(20.0), Some(50.0), Freshness::Reporting),
        )]);
        overview.outdoor = outdoor(Freshness::NotReporting);
        let html = page(&overview).await;
        assert!(html.contains("Weather unavailable"));
        assert!(html.contains("Not reporting"));
        assert!(!html.contains("Overcast"));
        assert!(html.contains("bedroom"));
        assert!(html.contains(r#"<span class="metric-value">20.0</span>"#));
    }

    #[tokio::test]
    async fn the_page_has_the_shell_with_overview_current() {
        let html = page(&overview(Vec::new())).await;
        assert_shell(&html, Page::Overview);
        assert!(html.contains(r#"<html lang="en" data-refresh="60">"#));
    }

    #[tokio::test]
    async fn alerts_come_before_rooms_and_rooms_before_outside() {
        let overview = overview(vec![room(
            "bedroom",
            reading(Some(20.0), Some(50.0), Freshness::Reporting),
        )]);
        let (_, html) = response(Some(&overview), Some(&active(1))).await;
        let alerts = html.find("Active alerts").expect("alerts heading");
        let rooms = html.find("Rooms and weather").expect("rooms heading");
        let bedroom = html.find("<h3>bedroom").expect("room card");
        let outside = html.find("<h3>Outside").expect("outside card");
        assert!(alerts < rooms && rooms < bedroom && bedroom < outside);
    }

    #[tokio::test]
    async fn an_unreachable_alerter_keeps_the_rooms_with_status_200() {
        let overview = overview(vec![room(
            "bedroom",
            reading(Some(20.0), Some(50.0), Freshness::Reporting),
        )]);
        let (status, html) = response(Some(&overview), None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Alerts unavailable"));
        assert!(html.contains(r#"<span class="metric-value">20.0</span>"#));
    }

    #[tokio::test]
    async fn an_unreachable_store_shows_the_notice_with_status_502() {
        let (status, html) = response(None, Some(&active(1))).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Room values unavailable"));
        assert!(html.contains("Updated 13:30"));
        assert!(html.contains("room1"));
        assert!(!html.contains("Rooms and weather"));
        assert_shell(&html, Page::Overview);
    }

    #[tokio::test]
    async fn more_than_three_alerts_show_the_two_oldest_and_a_link() {
        let (_, html) = response(Some(&overview(Vec::new())), Some(&active(5))).await;
        assert_eq!(html.matches(r#"class="alert-subject""#).count(), 2);
        assert!(html.contains(">room2<"));
        assert!(!html.contains(">room3<"));
        assert!(html.contains("3 more active alerts"));
        assert!(html.contains(r#"href="/alerts""#));
    }

    #[tokio::test]
    async fn three_alerts_show_all_three() {
        let (_, html) = response(Some(&overview(Vec::new())), Some(&active(3))).await;
        assert_eq!(html.matches(r#"class="alert-subject""#).count(), 3);
        assert!(!html.contains("more active alerts"));
        assert!(html.contains("Too hot"));
        assert!(html.contains("since 2026-01-15 13:30"));
    }

    #[tokio::test]
    async fn no_active_alerts_says_so() {
        let (_, html) = response(Some(&overview(Vec::new())), Some(&active(0))).await;
        assert!(html.contains("No active alerts."));
        assert!(!html.contains("alert-list alert-list--compact"));
    }
}
