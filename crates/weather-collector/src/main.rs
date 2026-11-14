use std::time::Duration;

use anyhow::{Context, anyhow};
use axum::{Router, routing::get};
use config::{MqttConfig, WeatherCollectorConfig};
use domain::SourceId;
use mqtt::MqttPublisher;
use open_meteo::OpenMeteo;
use tokio::net::TcpListener;
use use_cases::collect_weather;

const CLIENT_ID: &str = "weather-collector";
const SOURCE_ID: &str = "open-meteo";
const POLL_PERIOD: Duration = Duration::from_secs(15 * 60);

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = config::load()?;
    let weather = config
        .weather_collector
        .as_ref()
        .context("config has no [weather_collector] section")?;
    run(&config.mqtt, weather).await
}

async fn run(mqtt: &MqttConfig, weather: &WeatherCollectorConfig) -> anyhow::Result<()> {
    let listener = TcpListener::bind(weather.http_listen).await?;
    let health = Router::new().route("/healthz", get(|| async { "ok" }));
    let provider = OpenMeteo::new(weather.latitude, weather.longitude);
    let publisher = MqttPublisher::connect(&mqtt.host, mqtt.port, CLIENT_ID);
    let source_id = SourceId::from(SOURCE_ID.to_owned());
    tokio::select! {
        served = axum::serve(listener, health) => Err(anyhow!("health server ended: {served:?}")),
        never = collect_weather(&provider, &publisher, &source_id, POLL_PERIOD) => match never {},
    }
}
