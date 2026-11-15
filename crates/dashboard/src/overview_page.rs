use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;
use use_cases::{Freshness, Overview};

use crate::page::{self, Page};

#[derive(Template)]
#[template(path = "overview.html")]
struct OverviewPage<'a> {
    overview: &'a Overview,
    page: Page,
}

pub fn render(overview: &Overview) -> Response {
    let page = OverviewPage {
        overview,
        page: Page::Overview,
    };
    page::respond(&page, StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use domain::{MacAddress, RoomName, SourceId};
    use use_cases::{
        ClimateReading, Freshness, OutdoorOverview, Overview, RoomOverview, SensorOverview,
    };

    use crate::page::{Page, assert_shell};

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

    async fn page(overview: &Overview) -> String {
        let bytes = to_bytes(render(overview).into_body(), usize::MAX)
            .await
            .expect("body reads");
        String::from_utf8(bytes.to_vec()).expect("utf-8 body")
    }

    fn room(name: &str, reading: ClimateReading) -> RoomOverview {
        RoomOverview {
            room: RoomName::try_from(name.to_owned()).expect("valid room"),
            reading,
        }
    }

    #[tokio::test]
    async fn page_shows_rooms_and_unknown_sensors() {
        let overview = Overview {
            outdoor: outdoor(Freshness::Reporting),
            rooms: vec![
                room(
                    "bedroom",
                    reading(Some(21.27), Some(51.49), Freshness::Reporting),
                ),
                room("office", reading(None, None, Freshness::NotReporting)),
            ],
            unknown_sensors: vec![SensorOverview {
                source_id: SourceId::from(MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9])),
                reading: reading(Some(18.0), None, Freshness::Reporting),
            }],
        };
        let html = page(&overview).await;
        assert!(html.contains("bedroom"));
        assert!(html.contains("21.3 °C"));
        assert!(html.contains("51.5 %"));
        assert!(html.contains("not reporting"));
        assert!(html.contains("Unknown sensors"));
        assert!(html.contains("A4:C1:38:00:00:09"));
    }

    #[tokio::test]
    async fn unknown_section_is_hidden_when_empty() {
        let overview = Overview {
            outdoor: outdoor(Freshness::Reporting),
            rooms: vec![room(
                "bedroom",
                reading(Some(20.0), Some(50.0), Freshness::Reporting),
            )],
            unknown_sensors: Vec::new(),
        };
        let html = page(&overview).await;
        assert!(!html.contains("Unknown sensors"));
        assert!(!html.contains("not reporting"));
    }

    #[tokio::test]
    async fn available_weather_shows_the_outside_section() {
        let overview = Overview {
            outdoor: outdoor(Freshness::Reporting),
            rooms: Vec::new(),
            unknown_sensors: Vec::new(),
        };
        let html = page(&overview).await;
        assert!(html.contains("Outside"));
        assert!(html.contains("4.8 °C"));
        assert!(html.contains("Overcast"));
        assert!(html.contains("2.4–11.0 °C"));
        assert!(!html.contains("Weather unavailable"));
    }

    #[tokio::test]
    async fn unavailable_weather_keeps_the_rooms() {
        let overview = Overview {
            outdoor: outdoor(Freshness::NotReporting),
            rooms: vec![room(
                "bedroom",
                reading(Some(20.0), Some(50.0), Freshness::Reporting),
            )],
            unknown_sensors: Vec::new(),
        };
        let html = page(&overview).await;
        assert!(html.contains("Weather unavailable"));
        assert!(!html.contains("Overcast"));
        assert!(html.contains("bedroom"));
        assert!(html.contains("20.0 °C"));
    }

    #[tokio::test]
    async fn the_page_has_the_shell_with_overview_current() {
        let overview = Overview {
            outdoor: outdoor(Freshness::Reporting),
            rooms: Vec::new(),
            unknown_sensors: Vec::new(),
        };
        let html = page(&overview).await;
        assert_shell(&html, Page::Overview);
        assert!(html.contains("setInterval"));
        assert!(html.contains("}, 60 * 1000);"));
    }
}
