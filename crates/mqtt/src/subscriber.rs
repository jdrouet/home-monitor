use std::time::Duration;

use domain::{Measurement, batch, topic};
use rumqttc::{AsyncClient, ConnectionError, Event, Incoming, MqttOptions, Publish, QoS};
use tokio::time::sleep;
use tracing::warn;

use crate::SubscribeError;

const REQUEST_CAPACITY: usize = 64;
const KEEP_ALIVE: Duration = Duration::from_secs(30);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// Delivers every received measurement batch to `on_batch`, until a handler
/// or acknowledgement failure, which it returns.
///
/// The session is persistent under a fixed `client_id`. A batch is
/// acknowledged only after `on_batch` returned `Ok`, so the broker redelivers
/// anything not handled, including batches published while this client is
/// away. Invalid payloads are acknowledged and dropped.
pub async fn subscribe<F, E>(
    host: &str,
    port: u16,
    client_id: &str,
    mut on_batch: F,
) -> SubscribeError<E>
where
    F: FnMut(Vec<Measurement>) -> Result<(), E>,
    E: std::error::Error + 'static,
{
    let mut options = MqttOptions::new(client_id, host, port);
    options
        .set_clean_session(false)
        .set_manual_acks(true)
        .set_keep_alive(KEEP_ALIVE);
    let (client, mut event_loop) = AsyncClient::new(options, REQUEST_CAPACITY);
    loop {
        let polled = event_loop.poll().await;
        if let Err(error) = step(&client, polled, &mut on_batch).await {
            return error;
        }
    }
}

async fn step<F, E>(
    client: &AsyncClient,
    polled: Result<Event, ConnectionError>,
    on_batch: &mut F,
) -> Result<(), SubscribeError<E>>
where
    F: FnMut(Vec<Measurement>) -> Result<(), E>,
    E: std::error::Error + 'static,
{
    match polled {
        Ok(event) => handle_event(client, event, on_batch).await,
        Err(error) => {
            warn!(%error, "mqtt connection error");
            sleep(RECONNECT_DELAY).await;
            Ok(())
        }
    }
}

async fn handle_event<F, E>(
    client: &AsyncClient,
    event: Event,
    on_batch: &mut F,
) -> Result<(), SubscribeError<E>>
where
    F: FnMut(Vec<Measurement>) -> Result<(), E>,
    E: std::error::Error + 'static,
{
    match event {
        Event::Incoming(Incoming::ConnAck(_)) => {
            subscribe_measurements(client).await;
            Ok(())
        }
        Event::Incoming(Incoming::Publish(publish)) => on_publish(client, &publish, on_batch),
        _ => Ok(()),
    }
}

async fn subscribe_measurements(client: &AsyncClient) {
    if let Err(error) = client
        .subscribe(topic::MEASUREMENT_TOPIC_FILTER, QoS::AtLeastOnce)
        .await
    {
        warn!(%error, "mqtt subscribe failed");
    }
}

fn on_publish<F, E>(
    client: &AsyncClient,
    publish: &Publish,
    on_batch: &mut F,
) -> Result<(), SubscribeError<E>>
where
    F: FnMut(Vec<Measurement>) -> Result<(), E>,
    E: std::error::Error + 'static,
{
    handle_payload(&publish.payload, on_batch).map_err(SubscribeError::Handler)?;
    client.try_ack(publish).map_err(SubscribeError::Ack)
}

fn handle_payload<F, E>(payload: &[u8], on_batch: &mut F) -> Result<(), E>
where
    F: FnMut(Vec<Measurement>) -> Result<(), E>,
{
    match batch::decode(payload) {
        Ok(measurements) => on_batch(measurements),
        Err(error) => {
            warn!(%error, "dropping invalid measurement batch");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use domain::{MacAddress, Measurement, Metric, SourceKind, Timestamp, batch};
    use rumqttc::{AsyncClient, MqttOptions, Publish, QoS};

    use super::{handle_payload, on_publish};
    use crate::SubscribeError;

    fn sample() -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(1),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([1, 2, 3, 4, 5, 6]).into(),
            metric: Metric::Temperature,
            value: 21.5,
        }
    }

    fn failure() -> io::Error {
        io::Error::other("disk full")
    }

    fn publish() -> Publish {
        Publish::new(
            "measurements/ble/x",
            QoS::AtLeastOnce,
            batch::encode(&[sample()]),
        )
    }

    #[test]
    fn valid_payload_reaches_the_callback_once() {
        let mut received = Vec::new();
        let outcome = handle_payload(&batch::encode(&[sample()]), &mut |ms| {
            received.push(ms);
            Ok::<(), io::Error>(())
        });
        assert!(outcome.is_ok());
        assert_eq!(received, [vec![sample()]]);
    }

    #[test]
    fn invalid_payload_is_dropped_and_counts_as_handled() {
        let mut calls = 0;
        let outcome = handle_payload(b"not json", &mut |_| {
            calls += 1;
            Ok::<(), io::Error>(())
        });
        assert!(outcome.is_ok());
        assert_eq!(calls, 0);
    }

    #[test]
    fn callback_error_is_returned() {
        let outcome = handle_payload(&batch::encode(&[sample()]), &mut |_| Err(failure()));
        assert_eq!(outcome.expect_err("handler fails").to_string(), "disk full");
    }

    #[test]
    fn handled_publish_is_acknowledged() {
        let (client, _event_loop) = AsyncClient::new(MqttOptions::new("t", "localhost", 1883), 10);
        let outcome = on_publish(&client, &publish(), &mut |_| Ok::<(), io::Error>(()));
        assert!(outcome.is_ok());
    }

    #[test]
    fn failed_handler_leaves_the_publish_unacknowledged() {
        let (client, _event_loop) = AsyncClient::new(MqttOptions::new("t", "localhost", 1883), 10);
        let outcome = on_publish(&client, &publish(), &mut |_| Err(failure()));
        assert!(matches!(outcome, Err(SubscribeError::Handler(_))));
    }

    #[test]
    fn ack_failure_is_returned() {
        let (client, event_loop) = AsyncClient::new(MqttOptions::new("t", "localhost", 1883), 10);
        drop(event_loop);
        let outcome = on_publish(&client, &publish(), &mut |_| Ok::<(), io::Error>(()));
        assert!(matches!(outcome, Err(SubscribeError::Ack(_))));
    }
}
