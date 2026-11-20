use std::convert::Infallible;
use std::env::{self, VarError};
use std::time::Duration;

use anyhow::{Context, anyhow};
use config::Config;
use domain::{AlertState, Timestamp};
use import_client::ImportClient;
use ntfy::{NtfyNotifier, NtfyTopic};
use paris_clock::paris_datetime;
use reqwest::Url;
use state_file::StateFile;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::{Instant, MissedTickBehavior, interval, interval_at};
use tracing::warn;
use use_cases::{AlertStateStore, Alerts, Notifier};

mod import_client;
mod router;

const CLIENT_ID: &str = "alerter";
const TOPIC_ENV: &str = "NTFY_TOPIC";
const CHECK_PERIOD: Duration = Duration::from_secs(60);
const POLL_PERIOD: Duration = Duration::from_secs(60);
const POLL_TIMEOUT: Duration = Duration::from_secs(5);

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

/// The client for the health collector's import status; absent without a
/// `[health_collector]` section. The URL is checked either way, so a bad one
/// stops the alerter at startup.
fn import_client(config: &Config) -> anyhow::Result<Option<ImportClient>> {
    let url = Url::parse(&config.alerter.health_collector_url)
        .and_then(|base| base.join("imports"))
        .context("[alerter] health_collector_url is not a valid URL")?;
    Ok(config
        .health_collector
        .as_ref()
        .map(|_| ImportClient::new(url, POLL_TIMEOUT)))
}

async fn run(config: Config, notifier: NtfyNotifier) -> anyhow::Result<()> {
    let imports = import_client(&config)?;
    let store = StateFile::new(config.alerter.state_file.clone());
    let alerts = Alerts::new(&config.thermometers, &store, Timestamp::now());
    let listener = TcpListener::bind(config.alerter.http_listen).await?;
    let (report, reader) = watch::channel(alerts.state(Timestamp::now()));
    tokio::select! {
        served = axum::serve(listener, router::router(reader)) => Err(anyhow!("http server ended: {served:?}")),
        error = mqtt::subscribe(&config.mqtt.host, config.mqtt.port, CLIENT_ID, |batch| {
            alerts.observe(&batch);
            Ok::<(), Infallible>(())
        }) => Err(error.into()),
        never = notify_every(&alerts, &notifier, &store, &report) => match never {},
        never = poll_imports(imports.as_ref(), &alerts, &notifier, &store) => match never {},
    }
}

/// Reads the import status every `POLL_PERIOD` and sends the notifications it
/// owes, beside the alert loop. Never ends; without a client it never wakes.
async fn poll_imports<N: Notifier, S: AlertStateStore>(
    client: Option<&ImportClient>,
    alerts: &Alerts,
    notifier: &N,
    store: &S,
) -> Infallible {
    let Some(client) = client else {
        match std::future::pending::<Infallible>().await {}
    };
    let mut ticker = interval(POLL_PERIOD);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        poll_once(client, alerts, notifier, store).await;
    }
}

/// `at` on the Europe/Paris clock, as the notifications show the next try.
fn paris_time(at: Timestamp) -> String {
    paris_datetime(at).map_or_else(
        || "an unknown time".to_owned(),
        |time| format!("{} Paris time", time.format("%Y-%m-%d %H:%M")),
    )
}

/// A failed poll is logged by category and skipped; it changes nothing else.
async fn poll_once<N: Notifier, S: AlertStateStore>(
    client: &ImportClient,
    alerts: &Alerts,
    notifier: &N,
    store: &S,
) {
    match client.status().await {
        Ok(status) => {
            alerts
                .notify_imports(&status, notifier, store, Timestamp::now, paris_time)
                .await;
        }
        Err(error) => warn!(%error, "cannot read the import status, skipping this poll"),
    }
}

/// Checks every `CHECK_PERIOD` and publishes the alert report after each pass.
/// A pass waits for its notifications, so the report is refreshed normally
/// every 60 s and later when ntfy is slow.
async fn notify_every<N: Notifier, S: AlertStateStore>(
    alerts: &Alerts,
    notifier: &N,
    store: &S,
    report: &watch::Sender<AlertState>,
) -> Infallible {
    let mut ticker = interval_at(Instant::now() + CHECK_PERIOD, CHECK_PERIOD);
    loop {
        ticker.tick().await;
        alerts.notify_due(notifier, store, Timestamp::now).await;
        report.send_replace(alerts.state(Timestamp::now()));
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

    use std::cell::RefCell;
    use std::future::{self, Future};
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::Duration;

    use axum::Router;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use domain::{
        AlertState, AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure,
        ImportJob, ImportStatus, Timestamp, import_report,
    };
    use tracing_subscriber::fmt::MakeWriter;
    use use_cases::{AlertStateStore, Alerts, Notification, Notifier};

    use crate::import_client::ImportClient;
    use crate::import_client::tests::serve;

    use super::{paris_time, poll_once};

    const SENTINEL: &str = "SENTINEL_BODY_TEXT";

    #[test]
    fn the_next_try_is_shown_in_paris_time() {
        let summer = Timestamp::from_micros(1_784_188_800_000_000);
        assert_eq!(paris_time(summer), "2026-07-16 10:00 Paris time");
        assert_eq!(
            paris_time(Timestamp::from_micros(i64::MAX)),
            "an unknown time"
        );
    }

    #[derive(Default)]
    struct FakeNotifier {
        sent: RefCell<Vec<Notification>>,
    }

    impl Notifier for FakeNotifier {
        type Error = io::Error;

        fn notify(
            &self,
            notification: &Notification,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send {
            self.sent.borrow_mut().push(notification.clone());
            future::ready(Ok(()))
        }
    }

    struct FakeStore;

    impl AlertStateStore for FakeStore {
        type Error = io::Error;

        fn load(&self) -> Result<AlertState, Self::Error> {
            Ok(AlertState::default())
        }

        fn save(&self, _state: &AlertState) -> Result<(), Self::Error> {
            Ok(())
        }
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

    async fn client_for(router: Router) -> ImportClient {
        ImportClient::new(serve(router).await, Duration::from_millis(200))
    }

    fn give_up_report() -> Vec<u8> {
        let now = Timestamp::now().as_micros();
        let (started, retry_at) = (
            Timestamp::from_micros(now),
            Timestamp::from_micros(now + 3_600_000_000),
        );
        let attempt = ImportAttempt {
            started,
            ended: Some(AttemptEnd {
                at: started,
                outcome: AttemptOutcome::Failed(ImportFailure::GarminDown),
                counts: ImportCounts::default(),
            }),
        };
        let mut status = ImportStatus::default();
        status.record(ImportJob::Workouts, attempt, retry_at, None);
        import_report::encode(&status)
    }

    fn alerts() -> Alerts {
        Alerts::new(&[], &FakeStore, Timestamp::now())
    }

    #[tokio::test]
    async fn a_poll_after_a_failed_one_notifies_the_give_up() {
        let calls = Arc::new(AtomicUsize::new(0));
        let report = give_up_report();
        let handler = get(move || {
            let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
            let report = report.clone();
            async move {
                if first {
                    (StatusCode::INTERNAL_SERVER_ERROR, "down").into_response()
                } else {
                    report.into_response()
                }
            }
        });
        let client = client_for(Router::new().route("/imports", handler)).await;
        let (alerts, notifier) = (alerts(), FakeNotifier::default());
        poll_once(&client, &alerts, &notifier, &FakeStore).await;
        assert!(notifier.sent.borrow().is_empty());
        poll_once(&client, &alerts, &notifier, &FakeStore).await;
        poll_once(&client, &alerts, &notifier, &FakeStore).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_malformed_report_logs_a_category_and_never_its_content_or_address() {
        let captured = capture();
        let body = format!(r#"{{"version":1,"jobs":"{SENTINEL}"}}"#);
        let handler = get(move || {
            let body = body.clone();
            async move { body }
        });
        let client = client_for(Router::new().route("/imports", handler)).await;
        let (alerts, notifier) = (alerts(), FakeNotifier::default());
        poll_once(&client, &alerts, &notifier, &FakeStore).await;
        let logs = String::from_utf8(captured.0.lock().expect("lock").clone()).expect("utf8");
        assert!(logs.contains("unusable answer"), "{logs}");
        assert!(!logs.contains(SENTINEL), "{logs}");
        assert!(!logs.contains("127.0.0.1"), "{logs}");
        assert!(notifier.sent.borrow().is_empty());
    }
}
