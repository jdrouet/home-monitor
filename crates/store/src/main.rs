mod router;

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use config::Config;
use domain::Timestamp;
use storage::ParquetStore;
use tokio::net::TcpListener;
use tokio::time::{Instant, interval_at};
use use_cases::MeasurementStore;

const CLIENT_ID: &str = "store";
const FLUSH_INTERVAL: Duration = Duration::from_secs(3600);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    run(config::load()?).await
}

async fn run(config: Config) -> anyhow::Result<()> {
    let store = Arc::new(ParquetStore::open(&config.store.data_dir)?);
    store.flush_due(Timestamp::now()).await?;
    let writer = Arc::clone(&store);
    let listener = TcpListener::bind(config.store.http_listen).await?;
    tokio::select! {
        served = axum::serve(listener, router::router(Arc::clone(&store))) => {
            Err(anyhow!("http server ended: {served:?}"))
        }
        error = mqtt::subscribe(&config.mqtt.host, config.mqtt.port, CLIENT_ID, move |batch| {
            writer.append(batch)
        }) => Err(error.into()),
        error = flush_hourly(&*store) => Err(error.into()),
    }
}

async fn flush_hourly<S: MeasurementStore>(store: &S) -> S::Error {
    let mut ticker = interval_at(Instant::now() + FLUSH_INTERVAL, FLUSH_INTERVAL);
    loop {
        ticker.tick().await;
        if let Err(error) = store.flush_due(Timestamp::now()).await {
            return error;
        }
    }
}
