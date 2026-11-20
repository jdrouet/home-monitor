mod alerter_client;
mod alerts_page;
mod health_days;
mod health_page;
mod history_page;
mod history_params;
mod import_client;
mod imports_page;
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
use import_client::ImportClient;
use imports_page::Imports;
use paris_clock::paris_day_start;
use reqwest::Url;
use store_client::{StoreClient, StoreClientError};
use tokio::net::TcpListener;
use tracing::error;
use use_cases::{
    CalendarDay, HEALTH_METRICS, build_alert_summary, build_health, build_history,
    build_import_summary, build_overview, history_query,
};

const HEALTH_DAYS: usize = 30;

struct AppState {
    store: StoreClient,
    alerter: AlerterClient,
    /// Absent without a `[health_collector]` section.
    imports: Option<ImportClient>,
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
    let imports_url = Url::parse(&config.dashboard.health_collector_url)?.join("imports")?;
    let state = Arc::new(AppState {
        store: StoreClient::new(store_url.join("latest")?, store_url.join("series")?),
        alerter: AlerterClient::new(alerter_url.join("alerts")?),
        imports: config
            .health_collector
            .as_ref()
            .map(|_| ImportClient::new(imports_url)),
        thermometers: config.thermometers.clone(),
    });
    Ok(Router::new()
        .route("/", get(overview))
        .route("/history", get(history))
        .route("/alerts", get(alerts))
        .route("/health", get(health))
        .route("/imports", get(imports))
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

async fn imports(State(state): State<Arc<AppState>>) -> Response {
    let Some(client) = &state.imports else {
        return imports_page::render(Imports::NotSetUp, Timestamp::now());
    };
    let status = client.status().await;
    let now = Timestamp::now();
    match status {
        Ok(status) => {
            let summary = build_import_summary(&status);
            imports_page::render(Imports::Report(&summary), now)
        }
        Err(error) => {
            error!(%error, "cannot read the import status from the health collector");
            imports_page::render(Imports::Unavailable, now)
        }
    }
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
    use std::io;
    use std::sync::{Arc, LazyLock, Mutex};

    use axum::Router;
    use axum::extract::{Query, State};
    use axum::http::StatusCode;
    use axum::routing::get;
    use config::Config;
    use reqwest::Url;
    use tracing_subscriber::fmt::MakeWriter;

    use crate::alerter_client::AlerterClient;
    use crate::import_client::ImportClient;
    use crate::page::{read, serve};
    use crate::store_client::StoreClient;

    use super::{AppState, alerts, app, health, history, imports, overview};

    /// A state whose store and alerter refuse every connection.
    fn unreachable_state() -> Arc<AppState> {
        let unreachable = Url::parse("http://127.0.0.1:1/").expect("valid url");
        Arc::new(AppState {
            store: StoreClient::new(unreachable.clone(), unreachable.clone()),
            alerter: AlerterClient::new(unreachable.clone()),
            imports: Some(ImportClient::new(unreachable)),
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

    #[tokio::test]
    async fn an_unreachable_collector_is_a_bad_gateway_with_a_notice() {
        let state = unreachable_state();
        let (status, html) = read(imports(State(state)).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Import status unavailable"));
    }

    #[tokio::test]
    async fn without_a_health_collector_nothing_is_fetched_and_the_page_says_so() {
        let mut state = unreachable_state();
        Arc::get_mut(&mut state).expect("sole owner").imports = None;
        let (status, html) = read(imports(State(state)).await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Garmin is not set up."));
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl MakeWriter<'_> for Captured {
        type Writer = Self;

        fn make_writer(&self) -> Self {
            self.clone()
        }
    }

    /// A global subscriber: a scoped one races with callsites that other
    /// tests already registered with no subscriber.
    fn capture() -> &'static Captured {
        static CAPTURED: LazyLock<Captured> = LazyLock::new(|| {
            let captured = Captured::default();
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(captured.clone())
                .init();
            captured
        });
        &CAPTURED
    }

    #[tokio::test]
    async fn a_malformed_collector_report_reaches_neither_the_page_nor_the_log() {
        let captured = capture();
        let body = r#"{"version":1,"jobs":"<script>sentinel</script>"}"#;
        let router = Router::new().route("/imports", get(move || async move { body }));
        let url = serve(router).await;
        let mut state = unreachable_state();
        Arc::get_mut(&mut state).expect("sole owner").imports = Some(ImportClient::new(url));
        let (status, html) = read(imports(State(state)).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Import status unavailable"));
        assert!(!html.contains("sentinel"));
        let logs = String::from_utf8(captured.0.lock().expect("lock").clone()).expect("utf8");
        assert!(logs.contains("unusable answer"), "{logs}");
        assert!(!logs.contains("sentinel"), "{logs}");
        let ours: Vec<_> = logs
            .lines()
            .filter(|line| line.contains("unusable answer"))
            .collect();
        assert!(
            ours.iter().all(|line| !line.contains("127.0.0.1")),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_good_collector_report_renders_the_jobs() {
        let status = domain::ImportStatus::default();
        let body = domain::import_report::encode(&status);
        let router = Router::new().route("/imports", get(move || async move { body }));
        let url = serve(router).await;
        let mut state = unreachable_state();
        Arc::get_mut(&mut state).expect("sole owner").imports = Some(ImportClient::new(url));
        let (status, html) = read(imports(State(state)).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(html.matches("Not run yet").count(), 5);
    }
}
