mod alerter_client;
mod alerts_page;
mod history_page;
mod history_params;
mod overview_page;
mod paris_time;
mod store_client;
mod uplot_assets;

use std::net::SocketAddr;
use std::sync::Arc;

use alerter_client::AlerterClient;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use config::Config;
use domain::{Thermometer, Timestamp};
use paris_clock::paris_day_start;
use reqwest::Url;
use store_client::StoreClient;
use tokio::net::TcpListener;
use tracing::error;
use use_cases::{build_alert_summary, build_history, build_overview, history_query};

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
    match state.store.latest().await {
        Ok(latest) => {
            let now = Timestamp::now();
            overview_page::render(&build_overview(
                &state.thermometers,
                &latest,
                now,
                paris_day_start(now),
            ))
        }
        Err(error) => {
            error!(%error, "cannot read latest values from the store");
            (StatusCode::BAD_GATEWAY, "store unavailable").into_response()
        }
    }
}

async fn history(
    State(state): State<Arc<AppState>>,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let selection = history_params::parse(&pairs, &state.thermometers);
    let now = Timestamp::now();
    match state.store.series(&history_query(&selection, now)).await {
        Ok(rows) => {
            let chart = build_history(&state.thermometers, &selection, now, &rows);
            history_page::render(&selection, &state.thermometers, &chart)
        }
        Err(error) => {
            error!(%error, "cannot read the series from the store");
            (StatusCode::BAD_GATEWAY, "store unavailable").into_response()
        }
    }
}

async fn alerts(State(state): State<Arc<AppState>>) -> Response {
    match state.alerter.alerts().await {
        Ok(alert_state) => {
            let summary = build_alert_summary(&state.thermometers, &alert_state);
            alerts_page::render(Some(&summary))
        }
        Err(error) => {
            error!(%error, "cannot read the alerts from the alerter");
            (StatusCode::BAD_GATEWAY, alerts_page::render(None)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::http::StatusCode;
    use config::Config;
    use reqwest::Url;

    use crate::alerter_client::AlerterClient;
    use crate::store_client::StoreClient;

    use super::{AppState, alerts, app};

    const EXAMPLE: &str = include_str!("../../../config.example.toml");

    #[test]
    fn the_app_builds_from_the_example_config() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        assert!(app(&config).is_ok());
    }

    #[tokio::test]
    async fn an_unreachable_alerter_is_a_bad_gateway_with_a_notice() {
        let unreachable = Url::parse("http://127.0.0.1:1/alerts").expect("valid url");
        let state = Arc::new(AppState {
            store: StoreClient::new(unreachable.clone(), unreachable.clone()),
            alerter: AlerterClient::new(unreachable),
            thermometers: Vec::new(),
        });
        let response = alerts(State(state)).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let html = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
        assert!(html.contains("Alerts unavailable"));
    }
}
