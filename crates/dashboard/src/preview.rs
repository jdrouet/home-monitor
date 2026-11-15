//! Writes every page and state as static HTML for a design review.

use std::fs;
use std::path::Path;

use axum::response::Response;
use chrono::{Days, NaiveDate};
use domain::{AlertKind, MacAddress, Metric, RoomName, SourceId, Thermometer, Timestamp};
use use_cases::{
    ActiveAlertLine, AlertSummary, ClimateReading, Freshness, HealthRow, HistoryChart,
    HistorySelection, OutdoorOverview, Overview, Period, ResolvedAlertLine, RoomOverview,
    SensorOverview,
};

use crate::page::{at, read};
use crate::{alerts_page, health_page, history_page, overview_page};

const ROOMS: [&str; 6] = [
    "Bedroom",
    "Kitchen",
    "Office",
    "Living room",
    "Bathroom",
    "Kids room",
];

fn now() -> Timestamp {
    at("2026-01-15T12:42:00Z")
}

fn room_name(name: &str) -> RoomName {
    RoomName::try_from(name.to_owned()).expect("valid room")
}

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

fn overview(weather: Freshness) -> Overview {
    let reporting = |offset: f64| {
        reading(
            Some(19.5 + offset),
            Some(48.0 + offset * 2.0),
            Freshness::Reporting,
        )
    };
    let rooms = ROOMS
        .iter()
        .zip(0u8..)
        .map(|(name, index)| RoomOverview {
            room: room_name(name),
            reading: if *name == "Office" {
                reading(None, None, Freshness::NotReporting)
            } else {
                reporting(f64::from(index) * 0.7)
            },
        })
        .collect();
    Overview {
        outdoor: OutdoorOverview {
            reading: reading(Some(4.8), Some(78.0), weather),
            temperature_min: Some(2.4),
            temperature_max: Some(11.0),
            summary: Some("Overcast"),
        },
        rooms,
        unknown_sensors: vec![SensorOverview {
            source_id: SourceId::from(MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9])),
            reading: reading(Some(18.0), None, Freshness::Reporting),
        }],
    }
}

fn alerts(active: usize) -> AlertSummary {
    let lines = [
        (
            "Bedroom",
            AlertKind::TooHot,
            Some(26.3),
            "2026-01-15T09:30:00Z",
        ),
        ("Office", AlertKind::Offline, None, "2026-01-15T07:05:00Z"),
        (
            "Garmin",
            AlertKind::GarminLogin,
            None,
            "2026-01-14T18:00:00Z",
        ),
        (
            "Kitchen",
            AlertKind::TooCold,
            Some(14.8),
            "2026-01-14T06:10:00Z",
        ),
        ("Bathroom", AlertKind::Offline, None, "2026-01-13T21:45:00Z"),
    ];
    AlertSummary {
        active: lines
            .into_iter()
            .take(active)
            .map(|(subject, kind, celsius, since)| ActiveAlertLine {
                subject: subject.to_owned(),
                kind,
                celsius,
                since: at(since),
            })
            .collect(),
        resolved: vec![
            ResolvedAlertLine {
                subject: "Living room".to_owned(),
                kind: AlertKind::TooCold,
                started: at("2026-01-10T01:00:00Z"),
                ended: at("2026-01-10T07:30:00Z"),
            },
            ResolvedAlertLine {
                subject: "Garmin".to_owned(),
                kind: AlertKind::GarminLogin,
                started: at("2026-01-03T08:00:00Z"),
                ended: at("2026-01-04T08:00:00Z"),
            },
        ],
    }
}

fn thermometers() -> Vec<Thermometer> {
    ["Bedroom", "Kitchen", "Office"]
        .iter()
        .zip(1..)
        .map(|(name, last)| Thermometer {
            mac: MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, last]),
            room: room_name(name),
            temperature_range: None,
        })
        .collect()
}

fn selection() -> HistorySelection {
    HistorySelection {
        metric: Metric::Temperature,
        period: Period::Day,
        rooms: ["Bedroom", "Kitchen", "Office"]
            .iter()
            .map(|name| room_name(name))
            .collect(),
        include_outside: true,
    }
}

fn chart(with_readings: bool) -> HistoryChart {
    let end = now().as_micros() / 1_000_000;
    let timestamps: Vec<i64> = (0..96).rev().map(|step| end - step * 900).collect();
    let labels = ["Bedroom", "Kitchen", "Office", "Outside"];
    let lines = labels
        .iter()
        .zip(0u8..)
        .map(|(_, line)| {
            (0..96u32)
                .map(|step| {
                    let wave = (f64::from(step) / 96.0 * std::f64::consts::TAU).sin();
                    let value = 18.0 + f64::from(line) * 1.2 + wave * (1.0 + f64::from(line));
                    (with_readings && !(40..46).contains(&step)).then_some(value)
                })
                .collect()
        })
        .collect();
    HistoryChart {
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        timestamps,
        lines,
    }
}

fn health_rows(with_figures: bool) -> Vec<HealthRow> {
    let last = NaiveDate::from_ymd_opt(2026, 1, 15).expect("valid date");
    (0..30u32)
        .map(|offset| {
            let gap = !with_figures || offset % 9 == 4;
            let wave = f64::from(offset % 7);
            let figure = |value: f64| (!gap).then_some(value);
            HealthRow {
                date: last
                    .checked_sub_days(Days::new(u64::from(offset)))
                    .expect("valid date"),
                sleep_seconds: figure(25_200.0 + wave * 900.0),
                sleep_score: figure(70.0 + wave * 3.0),
                resting_heart_rate: figure(50.0 + wave),
                steps: figure(6_000.0 + wave * 1_100.0),
                body_battery_max: figure(90.0 + wave),
                body_battery_min: (offset % 11 != 3 && !gap).then_some(12.0 + wave),
            }
        })
        .collect()
}

fn write_bytes(root: &Path, relative: &str, contents: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("path has a parent")).expect("directory is created");
    fs::write(path, contents).expect("file is written");
}

#[tokio::test]
#[ignore = "writes preview HTML; set PREVIEW_DIR"]
async fn write_preview_pages() {
    let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR is set");
    let root = Path::new(&dir);
    let rooms = thermometers();
    let selected = selection();
    let full = overview(Freshness::Reporting);
    let day = now();
    let nothing = AlertSummary {
        active: Vec::new(),
        resolved: Vec::new(),
    };
    let health = health_rows(true);
    let no_figures = health_rows(false);
    let weather_down = overview(Freshness::NotReporting);

    let pages: [(&str, Response); 15] = [
        (
            "index.html",
            overview_page::render(Some(&full), Some(&alerts(3)), day),
        ),
        (
            "history/index.html",
            history_page::render(&selected, &rooms, Some(&chart(true))),
        ),
        (
            "alerts/index.html",
            alerts_page::render(Some(&alerts(3)), day),
        ),
        ("health/index.html", health_page::render(Some(&health))),
        (
            "states/overview-5-alerts.html",
            overview_page::render(Some(&full), Some(&alerts(5)), day),
        ),
        (
            "states/overview-alerter-down.html",
            overview_page::render(Some(&full), None, day),
        ),
        (
            "states/overview-store-down.html",
            overview_page::render(None, Some(&alerts(3)), day),
        ),
        (
            "states/overview-both-down.html",
            overview_page::render(None, None, day),
        ),
        (
            "states/overview-weather-down.html",
            overview_page::render(Some(&weather_down), Some(&alerts(0)), day),
        ),
        (
            "states/alerts-empty.html",
            alerts_page::render(Some(&nothing), day),
        ),
        ("states/alerts-down.html", alerts_page::render(None, day)),
        (
            "states/history-empty.html",
            history_page::render(&selected, &rooms, Some(&chart(false))),
        ),
        (
            "states/history-down.html",
            history_page::render(&selected, &rooms, None),
        ),
        (
            "states/health-empty.html",
            health_page::render(Some(&no_figures)),
        ),
        ("states/health-down.html", health_page::render(None)),
    ];
    for (path, response) in pages {
        let (_, html) = read(response).await;
        write_bytes(root, path, html.as_bytes());
    }

    write_bytes(
        root,
        "assets/uplot-1.6.32.min.js",
        include_bytes!("../assets/uPlot.iife.min.js"),
    );
    write_bytes(
        root,
        "assets/uplot-1.6.32.min.css",
        include_bytes!("../assets/uPlot.min.css"),
    );
}
