mod router;

use std::io;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use tokio::net::TcpListener;
use use_cases::LatestValues;

const CLIENT_ID: &str = "store";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = config::load()?;
    let state = Arc::new(Mutex::new(LatestValues::default()));
    let writer = Arc::clone(&state);
    let listener = TcpListener::bind(config.store.http_listen).await?;
    tokio::select! {
        served = axum::serve(listener, router::router(state)) => {
            Err(anyhow!("http server ended: {served:?}"))
        }
        error = mqtt::subscribe(&config.mqtt.host, config.mqtt.port, CLIENT_ID, move |batch| {
            writer.lock().expect("state lock poisoned").record(batch);
            Ok::<(), io::Error>(())
        }) => Err(error.into()),
    }
}
