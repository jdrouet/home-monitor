use std::time::Duration;

use domain::{Measurement, SourceId, SourceKind, batch, topic};
use rumqttc::{AsyncClient, ClientError, MqttOptions, QoS};
use tokio::time::sleep;
use tracing::warn;
use use_cases::Publisher;

const REQUEST_CAPACITY: usize = 64;
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// Publishes measurement batches to the broker at QoS 1.
pub struct MqttPublisher {
    client: AsyncClient,
}

impl MqttPublisher {
    /// Starts a reconnecting connection task on the current tokio runtime.
    /// The session persists, so unacked publishes survive a reconnect;
    /// publishing fails fast when the request queue is full.
    pub fn connect(host: &str, port: u16, client_id: &str) -> Self {
        let mut options = MqttOptions::new(client_id, host, port);
        options.set_clean_session(false);
        let (client, mut event_loop) = AsyncClient::new(options, REQUEST_CAPACITY);
        tokio::spawn(async move {
            loop {
                if let Err(error) = event_loop.poll().await {
                    warn!(%error, "mqtt connection error");
                    sleep(RECONNECT_DELAY).await;
                }
            }
        });
        Self { client }
    }
}

impl Publisher for MqttPublisher {
    type Error = ClientError;

    async fn publish(
        &self,
        source_kind: SourceKind,
        source_id: &SourceId,
        measurements: &[Measurement],
    ) -> Result<(), ClientError> {
        self.client.try_publish(
            topic::measurement_topic(source_kind, source_id),
            QoS::AtLeastOnce,
            false,
            batch::encode(measurements),
        )
    }
}
