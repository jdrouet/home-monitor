mod history_page;
mod history_params;
mod overview_page;
mod paris_day_start;
mod store_client;
mod uplot_assets;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use config::Config;
use domain::{Thermometer, Timestamp};
use paris_day_start::paris_day_start;
use reqwest::Url;
use store_client::StoreClient;
use tokio::net::TcpListener;
use tracing::error;
use use_cases::{build_history, build_overview, history_query};

struct AppState {
    store: StoreClient,
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
    let state = Arc::new(AppState {
        store: StoreClient::new(store_url.join("latest")?, store_url.join("series")?),
        thermometers: config.thermometers.clone(),
    });
    Ok(Router::new()
        .route("/", get(overview))
        .route("/history", get(history))
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
