mod overview_page;
mod store_client;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use config::Config;
use domain::{Thermometer, Timestamp};
use reqwest::Url;
use store_client::StoreClient;
use tokio::net::TcpListener;
use tracing::error;
use use_cases::build_overview;

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
    let latest_url = Url::parse(&config.dashboard.store_url)?.join("latest")?;
    let state = Arc::new(AppState {
        store: StoreClient::new(latest_url),
        thermometers: config.thermometers.clone(),
    });
    Ok(Router::new()
        .route("/", get(overview))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state))
}

async fn serve(address: SocketAddr, app: Router) -> std::io::Result<()> {
    axum::serve(TcpListener::bind(address).await?, app).await
}

async fn overview(State(state): State<Arc<AppState>>) -> Response {
    match state.store.latest().await {
        Ok(latest) => overview_page::render(&build_overview(
            &state.thermometers,
            &latest,
            Timestamp::now(),
        )),
        Err(error) => {
            error!(%error, "cannot read latest values from the store");
            (StatusCode::BAD_GATEWAY, "store unavailable").into_response()
        }
    }
}
