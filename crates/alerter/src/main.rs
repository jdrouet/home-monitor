use std::convert::Infallible;
use std::env;
use std::time::Duration;

use anyhow::{Context, anyhow};
use axum::{Router, routing::get};
use config::Config;
use domain::Timestamp;
use ntfy::{NtfyNotifier, NtfyTopic};
use tokio::net::TcpListener;
use tokio::time::{Instant, interval_at};
use use_cases::{Notifier, OfflineAlerts};

const CLIENT_ID: &str = "alerter";
const TOPIC_ENV: &str = "NTFY_TOPIC";
const CHECK_PERIOD: Duration = Duration::from_secs(60);

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = config::load()?;
    let notifier = notifier(&config)?;
    run(config, notifier).await
}

fn notifier(config: &Config) -> anyhow::Result<NtfyNotifier> {
    let topic = env::var(TOPIC_ENV).context("NTFY_TOPIC is not set")?;
    let topic: NtfyTopic = topic.parse()?;
    Ok(NtfyNotifier::new(&config.alerter.ntfy_url, &topic)?)
}

async fn run(config: Config, notifier: NtfyNotifier) -> anyhow::Result<()> {
    let alerts = OfflineAlerts::new(&config.thermometers, Timestamp::now());
    let listener = TcpListener::bind(config.alerter.http_listen).await?;
    let health = Router::new().route("/healthz", get(|| async { "ok" }));
    tokio::select! {
        served = axum::serve(listener, health) => Err(anyhow!("health server ended: {served:?}")),
        error = mqtt::subscribe(&config.mqtt.host, config.mqtt.port, CLIENT_ID, |batch| {
            alerts.observe(&batch);
            Ok::<(), Infallible>(())
        }) => Err(error.into()),
        never = notify_every(&alerts, &notifier) => match never {},
    }
}

async fn notify_every<N: Notifier>(alerts: &OfflineAlerts, notifier: &N) -> Infallible {
    let mut ticker = interval_at(Instant::now() + CHECK_PERIOD, CHECK_PERIOD);
    loop {
        ticker.tick().await;
        alerts.notify_due(notifier, Timestamp::now).await;
    }
}
