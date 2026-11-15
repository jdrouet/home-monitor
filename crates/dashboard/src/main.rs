mod alerter_client;
mod alerts_page;
mod health_days;
mod health_page;
mod history_page;
mod history_params;
mod overview_page;
mod page;
mod paris_time;
#[cfg(test)]
mod preview;
mod store_client;
mod uplot_assets;

use std::net::SocketAddr;
use std::sync::Arc;

use alerter_client::AlerterClient;
use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use config::Config;
use domain::{Measurement, SeriesQuery, Thermometer, Timestamp};
use health_days::health_days;
use paris_clock::paris_day_start;
use reqwest::Url;
use store_client::{StoreClient, StoreClientError};
use tokio::net::TcpListener;
use tracing::error;
use use_cases::{
    CalendarDay, HEALTH_METRICS, build_alert_summary, build_health, build_history, build_overview,
    history_query,
};

const HEALTH_DAYS: usize = 30;

struct AppState {
    store: StoreClient,
    alerter: AlerterClient,
    thermometers: Vec<Thermometer>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = config::load()?;
    let app = app(&config)?;
    serve(config.dashboard.http_listen, app).await?;
    Ok(())
}

fn app(config: &Config) -> anyhow::Result<Router> {
    let store_url = Url::parse(&config.dashboard.store_url)?;
    let alerter_url = Url::parse(&config.dashboard.alerter_url)?;
    let state = Arc::new(AppState {
        store: StoreClient::new(store_url.join("latest")?, store_url.join("series")?),
        alerter: AlerterClient::new(alerter_url.join("alerts")?),
        thermometers: config.thermometers.clone(),
    });
    Ok(Router::new()
        .route("/", get(overview))
        .route("/history", get(history))
        .route("/alerts", get(alerts))
        .route("/health", get(health))
        .route("/healthz", get(|| async { "ok" }))
        .route("/assets/uplot-1.6.32.min.js", get(uplot_assets::script))
        .route(
            "/assets/uplot-1.6.32.min.css",
            get(uplot_assets::stylesheet),
        )
        .with_state(state))
}

async fn serve(address: SocketAddr, app: Router) -> std::io::Result<()> {
    axum::serve(TcpListener::bind(address).await?, app).await
}

async fn overview(State(state): State<Arc<AppState>>) -> Response {
    let now = Timestamp::now();
    let (latest, alerts) = tokio::join!(state.store.latest(), state.alerter.alerts());
    let overview = latest
        .inspect_err(|error| error!(%error, "cannot read latest values from the store"))
        .ok()
        .map(|latest| build_overview(&state.thermometers, &latest, now, paris_day_start(now)));
    let summary = alerts
        .inspect_err(|error| error!(%error, "cannot read the alerts from the alerter"))
        .ok()
        .map(|alerts| build_alert_summary(&state.thermometers, &alerts));
    overview_page::render(overview.as_ref(), summary.as_ref(), now)
}

async fn history(
    State(state): State<Arc<AppState>>,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let selection = history_params::parse(&pairs, &state.thermometers);
    let now = Timestamp::now();
    let chart = state
        .store
        .series(&history_query(&selection, now))
        .await
        .inspect_err(|error| error!(%error, "cannot read the series from the store"))
        .ok()
        .map(|rows| build_history(&state.thermometers, &selection, now, &rows));
    history_page::render(&selection, &state.thermometers, chart.as_ref())
}

async fn alerts(State(state): State<Arc<AppState>>) -> Response {
    let now = Timestamp::now();
    let summary = state
        .alerter
        .alerts()
        .await
        .inspect_err(|error| error!(%error, "cannot read the alerts from the alerter"))
        .ok()
        .map(|alert_state| build_alert_summary(&state.thermometers, &alert_state));
    alerts_page::render(summary.as_ref(), now)
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    let days = health_days(Timestamp::now(), HEALTH_DAYS);
    let rows = health_measurements(&state.store, &days)
        .await
        .inspect_err(|error| error!(%error, "cannot read the health series from the store"))
        .ok()
        .map(|measurements| build_health(&days, &measurements));
    health_page::render(rows.as_deref())
}

/// One store query per health metric over all of `days`, run in turn.
async fn health_measurements(
    store: &StoreClient,
    days: &[CalendarDay],
) -> Result<Vec<Measurement>, StoreClientError> {
    let from = days
        .last()
        .map_or(Timestamp::from_micros(0), |day| day.start);
    let to = days
        .first()
        .map_or(Timestamp::from_micros(0), |day| day.end);
    let mut measurements = Vec::new();
    for metric in HEALTH_METRICS {
        let query = SeriesQuery {
            metric,
            source_id: None,
            from,
            to,
            bucket_secs: None,
        };
        measurements.extend(store.series(&query).await?);
    }
    Ok(measurements)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::extract::{Query, State};
    use axum::http::StatusCode;
    use config::Config;
    use reqwest::Url;

    use crate::alerter_client::AlerterClient;
    use crate::page::read;
    use crate::store_client::StoreClient;

    use super::{AppState, alerts, app, health, history, overview};

    /// A state whose store and alerter refuse every connection.
    fn unreachable_state() -> Arc<AppState> {
        let unreachable = Url::parse("http://127.0.0.1:1/").expect("valid url");
        Arc::new(AppState {
            store: StoreClient::new(unreachable.clone(), unreachable.clone()),
            alerter: AlerterClient::new(unreachable),
            thermometers: Vec::new(),
        })
    }

    const EXAMPLE: &str = include_str!("../../../config.example.toml");

    #[test]
    fn the_app_builds_from_the_example_config() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        assert!(app(&config).is_ok());
    }

    #[tokio::test]
    async fn an_unreachable_alerter_is_a_bad_gateway_with_a_notice() {
        let state = unreachable_state();
        let (status, html) = read(alerts(State(state)).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Alerts unavailable"));
    }

    #[tokio::test]
    async fn an_unreachable_store_and_alerter_render_both_notices() {
        let state = unreachable_state();
        let (status, html) = read(overview(State(state)).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        let alerts = html.find("Alerts unavailable").expect("alerts notice");
        let rooms = html.find("Room values unavailable").expect("rooms notice");
        assert!(alerts < rooms);
    }

    #[tokio::test]
    async fn an_unreachable_store_is_a_bad_gateway_on_the_history_page() {
        let state = unreachable_state();
        let (status, html) = read(history(State(state), Query(Vec::new())).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("History unavailable"));
    }

    #[tokio::test]
    async fn an_unreachable_store_is_a_bad_gateway_on_the_health_page() {
        let state = unreachable_state();
        let (status, html) = read(health(State(state)).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Health data unavailable"));
    }
}
