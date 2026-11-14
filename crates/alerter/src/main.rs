use std::convert::Infallible;
use std::env::{self, VarError};
use std::time::Duration;

use anyhow::anyhow;
use axum::{Router, routing::get};
use config::Config;
use domain::Timestamp;
use ntfy::{NtfyNotifier, NtfyTopic};
use state_file::StateFile;
use tokio::net::TcpListener;
use tokio::time::{Instant, interval_at};
use use_cases::{AlertStateStore, Alerts, Notifier};

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
    let topic = topic_from_env(env::var(TOPIC_ENV))?;
    Ok(NtfyNotifier::new(&config.alerter.ntfy_url, &topic)?)
}

// The error must never carry the variable's value: the topic is a secret.
fn topic_from_env(value: Result<String, VarError>) -> anyhow::Result<NtfyTopic> {
    let value = value.map_err(|_| anyhow!("{TOPIC_ENV} is not set or not valid UTF-8"))?;
    Ok(value.parse()?)
}

async fn run(config: Config, notifier: NtfyNotifier) -> anyhow::Result<()> {
    let store = StateFile::new(config.alerter.state_file.clone());
    let alerts = Alerts::new(&config.thermometers, &store, Timestamp::now());
    let listener = TcpListener::bind(config.alerter.http_listen).await?;
    let health = Router::new().route("/healthz", get(|| async { "ok" }));
    tokio::select! {
        served = axum::serve(listener, health) => Err(anyhow!("health server ended: {served:?}")),
        error = mqtt::subscribe(&config.mqtt.host, config.mqtt.port, CLIENT_ID, |batch| {
            alerts.observe(&batch);
            Ok::<(), Infallible>(())
        }) => Err(error.into()),
        never = notify_every(&alerts, &notifier, &store) => match never {},
    }
}

async fn notify_every<N: Notifier, S: AlertStateStore>(
    alerts: &Alerts,
    notifier: &N,
    store: &S,
) -> Infallible {
    let mut ticker = interval_at(Instant::now() + CHECK_PERIOD, CHECK_PERIOD);
    loop {
        ticker.tick().await;
        alerts.notify_due(notifier, store, Timestamp::now).await;
    }
}

#[cfg(test)]
mod tests {
    use std::env::VarError;
    use std::ffi::OsString;

    use super::topic_from_env;

    #[test]
    fn non_unicode_value_is_not_leaked() {
        let error = topic_from_env(Err(VarError::NotUnicode(OsString::from("SECRET_SENTINEL"))))
            .expect_err("not unicode");
        assert!(!format!("{error:#}").contains("SECRET_SENTINEL"));
        assert!(!format!("{error:?}").contains("SECRET_SENTINEL"));
    }

    #[test]
    fn missing_variable_is_named() {
        let error = topic_from_env(Err(VarError::NotPresent)).expect_err("not set");
        assert!(error.to_string().contains("NTFY_TOPIC"));
    }

    #[test]
    fn invalid_topic_is_rejected_without_echo() {
        let error = topic_from_env(Ok("SECRET/SENTINEL".to_owned())).expect_err("invalid");
        assert!(!format!("{error:#}").contains("SECRET"));
        assert!(topic_from_env(Ok("good_topic".to_owned())).is_ok());
    }
}
