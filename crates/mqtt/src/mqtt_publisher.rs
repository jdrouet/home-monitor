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
    /// Starts the connection task on the current tokio runtime. The task
    /// reconnects forever, so publishing never needs the broker to be up first.
    pub fn connect(host: &str, port: u16, client_id: &str) -> Self {
        let (client, mut event_loop) =
            AsyncClient::new(MqttOptions::new(client_id, host, port), REQUEST_CAPACITY);
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
        self.client
            .publish(
                topic::measurement_topic(source_kind, source_id),
                QoS::AtLeastOnce,
                false,
                batch::encode(measurements),
            )
            .await
    }
}
