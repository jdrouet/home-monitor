use std::time::Duration;

use anyhow::anyhow;
use axum::{Router, routing::get};
use mqtt::MqttPublisher;
use tokio::net::TcpListener;
use use_cases::Collector;

const CLIENT_ID: &str = "ble-collector";
const FLUSH_PERIOD: Duration = Duration::from_secs(60);

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = config::load()?;
    let publisher = MqttPublisher::connect(&config.mqtt.host, config.mqtt.port, CLIENT_ID);
    let source = bluez::start().await?;
    let listener = TcpListener::bind(config.ble_collector.http_listen).await?;
    let health = Router::new().route("/healthz", get(|| async { "ok" }));
    tokio::select! {
        served = axum::serve(listener, health) => Err(anyhow!("health server ended: {served:?}")),
        () = Collector::default().run(source, &publisher, FLUSH_PERIOD) => {
            Err(anyhow!("advertisement source ended"))
        }
    }
}
