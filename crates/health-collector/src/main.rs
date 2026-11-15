mod command;
mod prompt;
mod schedule;

use std::convert::Infallible;
use std::env;
use std::io::{self, BufRead, Write};

use anyhow::{Context, anyhow};
use axum::{Router, routing::get};
use command::Command;
use config::{HealthCollectorConfig, MqttConfig};
use domain::{SourceId, Timestamp};
use garmin_connect::{GarminConnect, TokenFile};
use mqtt::MqttPublisher;
use paris_clock::paris_date;
use prompt::prompt_line;
use schedule::{completed_day_stamp, next_pull, pull_days, wait_until};
use state_file::BackfillFile;
use tokio::net::TcpListener;
use tokio::time::sleep;
use use_cases::{
    BackfillStore, HealthBackfill, HealthProvider, HealthPull, Publisher, collect_health,
};

const CLIENT_ID: &str = "health-collector";
const SOURCE_ID: &str = "garmin";
const USAGE: &str = "usage: health-collector [login]";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let command = Command::parse(env::args().skip(1)).ok_or_else(|| anyhow!(USAGE))?;
    let config = config::load()?;
    let health = config
        .health_collector
        .as_ref()
        .context("config has no [health_collector] section")?;
    dispatch(command, &config.mqtt, health).await
}

async fn dispatch(
    command: Command,
    mqtt: &MqttConfig,
    health: &HealthCollectorConfig,
) -> anyhow::Result<()> {
    match command {
        Command::Login => login(health).await,
        Command::Collect => collect(mqtt, health).await,
    }
}

async fn login(health: &HealthCollectorConfig) -> anyhow::Result<()> {
    let garmin = GarminConnect::new(TokenFile::new(health.token_file.clone()))?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout();
    let (email, password) = read_credentials(&mut input, &mut output)?;
    garmin
        .login(&email, &password, || {
            prompt_line(&mut input, &mut output, "MFA code")
        })
        .await?;
    println!("Logged in; the tokens are saved.");
    Ok(())
}

fn read_credentials(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<(String, String)> {
    let email = prompt_line(input, output, "Garmin email")?;
    let password = rpassword::prompt_password("Garmin password: ")?;
    Ok((email, password))
}

async fn collect(mqtt: &MqttConfig, health: &HealthCollectorConfig) -> anyhow::Result<()> {
    let listener = TcpListener::bind(health.http_listen).await?;
    let router = Router::new().route("/healthz", get(|| async { "ok" }));
    let provider = GarminConnect::new(TokenFile::new(health.token_file.clone()))?;
    let publisher = MqttPublisher::connect(&mqtt.host, mqtt.port, CLIENT_ID);
    let backfill_store =
        BackfillFile::new(health.token_file.with_file_name("garmin_backfill.json"));
    let source_id = SourceId::from(SOURCE_ID.to_owned());
    tokio::select! {
        served = axum::serve(listener, router) => Err(anyhow!("health server ended: {served:?}")),
        never = pull_forever(&provider, &publisher, &backfill_store, &source_id) => match never {},
    }
}

/// Pulls at startup, then at every daily pull time. The next deadline is
/// fixed before a pull starts, so a slow pull cannot skip it. A failed pull
/// is not retried before the next one. After a completed pull the backfill
/// runs until shortly before the next deadline.
async fn pull_forever<H: HealthProvider, P: Publisher, S: BackfillStore>(
    provider: &H,
    publisher: &P,
    backfill_store: &S,
    source_id: &SourceId,
) -> Infallible {
    loop {
        let started = Timestamp::now();
        let next = next_pull(started);
        let pull = collect_health(provider, publisher, source_id, &pull_days(started)).await;
        if matches!(pull, HealthPull::Completed { .. }) {
            HealthBackfill {
                provider,
                publisher,
                store: backfill_store,
                source_id,
            }
            .run(
                paris_date(started),
                next,
                Timestamp::now,
                completed_day_stamp,
            )
            .await;
        }
        sleep(wait_until(next, Timestamp::now())).await;
    }
}
