/// Why `subscribe` stopped. Either way the batch is left unacknowledged, so
/// the broker redelivers it to the next session.
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError<E: std::error::Error + 'static> {
    #[error("measurement handler failed")]
    Handler(#[source] E),
    #[error("cannot acknowledge batch")]
    Ack(#[source] rumqttc::ClientError),
}
