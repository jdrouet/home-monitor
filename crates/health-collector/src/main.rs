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
use prompt::prompt_line;
use schedule::{next_pull, pull_days};
use tokio::net::TcpListener;
use tokio::time::sleep;
use use_cases::{HealthProvider, Publisher, collect_health};

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
    let source_id = SourceId::from(SOURCE_ID.to_owned());
    tokio::select! {
        served = axum::serve(listener, router) => Err(anyhow!("health server ended: {served:?}")),
        never = pull_forever(&provider, &publisher, &source_id) => match never {},
    }
}

/// Pulls at startup, then at every daily pull time. A failed pull is not
/// retried before the next one.
async fn pull_forever<H: HealthProvider, P: Publisher>(
    provider: &H,
    publisher: &P,
    source_id: &SourceId,
) -> Infallible {
    loop {
        let started = Timestamp::now();
        collect_health(provider, publisher, source_id, &pull_days(started)).await;
        let finished = Timestamp::now();
        sleep(next_pull(finished).duration_since(finished)).await;
    }
}
