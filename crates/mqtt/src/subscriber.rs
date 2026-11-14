use std::time::Duration;

use domain::{Measurement, batch, topic};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use tokio::time::sleep;
use tracing::warn;

const REQUEST_CAPACITY: usize = 64;
const KEEP_ALIVE: Duration = Duration::from_secs(30);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// Delivers every received measurement batch to `on_batch`, forever.
///
/// The session is persistent under a fixed `client_id`, so the broker queues
/// batches published while this client is away.
pub async fn subscribe<F: FnMut(Vec<Measurement>)>(
    host: &str,
    port: u16,
    client_id: &str,
    mut on_batch: F,
) -> ! {
    let mut options = MqttOptions::new(client_id, host, port);
    options.set_clean_session(false).set_keep_alive(KEEP_ALIVE);
    let (client, mut event_loop) = AsyncClient::new(options, REQUEST_CAPACITY);
    loop {
        match event_loop.poll().await {
            Ok(event) => handle_event(&client, event, &mut on_batch).await,
            Err(error) => {
                warn!(%error, "mqtt connection error");
                sleep(RECONNECT_DELAY).await;
            }
        }
    }
}

async fn handle_event<F: FnMut(Vec<Measurement>)>(
    client: &AsyncClient,
    event: Event,
    on_batch: &mut F,
) {
    match event {
        Event::Incoming(Incoming::ConnAck(_)) => {
            if let Err(error) = client
                .subscribe(topic::MEASUREMENT_TOPIC_FILTER, QoS::AtLeastOnce)
                .await
            {
                warn!(%error, "mqtt subscribe failed");
            }
        }
        Event::Incoming(Incoming::Publish(publish)) => handle_payload(&publish.payload, on_batch),
        _ => {}
    }
}

fn handle_payload<F: FnMut(Vec<Measurement>)>(payload: &[u8], on_batch: &mut F) {
    match batch::decode(payload) {
        Ok(measurements) => on_batch(measurements),
        Err(error) => warn!(%error, "dropping invalid measurement batch"),
    }
}

#[cfg(test)]
mod tests {
    use domain::{MacAddress, Measurement, Metric, SourceKind, Timestamp, batch};

    use super::handle_payload;

    fn sample() -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(1),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([1, 2, 3, 4, 5, 6]).into(),
            metric: Metric::Temperature,
            value: 21.5,
        }
    }

    #[test]
    fn valid_payload_reaches_the_callback_once() {
        let mut received = Vec::new();
        handle_payload(&batch::encode(&[sample()]), &mut |ms| received.push(ms));
        assert_eq!(received, [vec![sample()]]);
    }

    #[test]
    fn invalid_payload_is_dropped() {
        let mut calls = 0;
        handle_payload(b"not json", &mut |_| calls += 1);
        assert_eq!(calls, 0);
    }
}
