use domain::{Measurement, Metric, SourceId, SourceKind, Timestamp};
use tracing::warn;

use crate::{HealthPull, Publisher};

/// Publishes whether the Garmin session worked: 1 after a completed pull, 0
/// after a failed session. A throttled or failed pull says nothing about the
/// login, so nothing is published.
pub async fn report_session<P: Publisher>(
    publisher: &P,
    source_id: &SourceId,
    pull: &HealthPull,
    at: Timestamp,
) {
    let value = match pull {
        HealthPull::Completed { .. } => 1.0,
        HealthPull::SessionFailed => 0.0,
        HealthPull::Throttled | HealthPull::Failed => return,
    };
    let measurement = Measurement {
        timestamp: at,
        source_kind: SourceKind::Health,
        source_id: source_id.clone(),
        metric: Metric::SessionValid,
        value,
    };
    if let Err(error) = publisher
        .publish(SourceKind::Health, source_id, &[measurement])
        .await
    {
        warn!(%error, "session status publish failed");
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Mutex;

    use domain::{Measurement, Metric, SourceId, SourceKind, Timestamp};

    use crate::{HealthPull, Publisher};

    use super::report_session;

    #[derive(Default)]
    struct TestPublisher {
        failing: bool,
        published: Mutex<Vec<(SourceKind, SourceId, Vec<Measurement>)>>,
    }

    impl Publisher for TestPublisher {
        type Error = io::Error;

        async fn publish(
            &self,
            source_kind: SourceKind,
            source_id: &SourceId,
            measurements: &[Measurement],
        ) -> Result<(), io::Error> {
            if self.failing {
                return Err(io::Error::other("broker down"));
            }
            self.published.lock().expect("lock").push((
                source_kind,
                source_id.clone(),
                measurements.to_vec(),
            ));
            Ok(())
        }
    }

    async fn report(publisher: &TestPublisher, pull: HealthPull) {
        let source_id = SourceId::from("garmin".to_owned());
        report_session(publisher, &source_id, &pull, Timestamp::from_micros(7)).await;
    }

    #[tokio::test]
    async fn a_completed_pull_publishes_one_stamped_at_the_pull() {
        let publisher = TestPublisher::default();
        report(&publisher, HealthPull::Completed { fetched: vec![] }).await;
        let published = publisher.published.lock().expect("lock");
        assert_eq!(published.len(), 1);
        let (kind, id, batch) = &published[0];
        assert_eq!(*kind, SourceKind::Health);
        assert_eq!(id.as_str(), "garmin");
        assert_eq!(
            *batch,
            [Measurement {
                timestamp: Timestamp::from_micros(7),
                source_kind: SourceKind::Health,
                source_id: id.clone(),
                metric: Metric::SessionValid,
                value: 1.0,
            }]
        );
    }

    #[tokio::test]
    async fn a_failed_session_publishes_zero() {
        let publisher = TestPublisher::default();
        report(&publisher, HealthPull::SessionFailed).await;
        let published = publisher.published.lock().expect("lock");
        assert_eq!(published[0].2[0].value, 0.0);
        assert_eq!(published[0].2[0].metric, Metric::SessionValid);
    }

    #[tokio::test]
    async fn throttled_and_failed_pulls_publish_nothing() {
        let publisher = TestPublisher::default();
        report(&publisher, HealthPull::Throttled).await;
        report(&publisher, HealthPull::Failed).await;
        assert!(publisher.published.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn a_failed_publish_is_only_logged() {
        let publisher = TestPublisher {
            failing: true,
            ..TestPublisher::default()
        };
        report(&publisher, HealthPull::SessionFailed).await;
        assert!(publisher.published.lock().expect("lock").is_empty());
    }
}
