use std::convert::Infallible;
use std::time::Duration;

use domain::{SourceId, SourceKind};
use tokio::time::interval;
use tracing::warn;

use crate::{Publisher, WeatherProvider};

/// Polls `provider` every `period`, starting at once, and publishes each report.
/// A failed fetch or publish is logged and skipped; the next poll replaces it.
pub async fn collect_weather<W: WeatherProvider, P: Publisher>(
    provider: &W,
    publisher: &P,
    source_id: &SourceId,
    period: Duration,
) -> Infallible {
    let mut ticker = interval(period);
    loop {
        ticker.tick().await;
        collect_once(provider, publisher, source_id).await;
    }
}

async fn collect_once<W: WeatherProvider, P: Publisher>(
    provider: &W,
    publisher: &P,
    source_id: &SourceId,
) {
    let report = match provider.current().await {
        Ok(report) => report,
        Err(error) => {
            warn!(%error, "weather fetch failed, skipping this poll");
            return;
        }
    };
    let batch = report.measurements(source_id);
    if let Err(error) = publisher
        .publish(SourceKind::Weather, source_id, &batch)
        .await
    {
        warn!(%error, "weather publish failed, skipping this poll");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::Mutex;
    use std::time::Duration;

    use domain::{Measurement, SourceId, SourceKind, Timestamp};
    use tokio::time::timeout;

    use crate::{Publisher, WeatherProvider, WeatherReport};

    use super::collect_weather;

    const PERIOD: Duration = Duration::from_secs(15 * 60);
    const RUN_FOR: Duration = Duration::from_secs(31 * 60);

    fn report() -> WeatherReport {
        WeatherReport {
            observed_at: Timestamp::from_micros(1),
            temperature: 4.8,
            humidity: 78.0,
            temperature_min: 2.4,
            temperature_max: 11.0,
            weather_code: 3,
        }
    }

    struct TestProvider(Mutex<VecDeque<Result<WeatherReport, io::Error>>>);

    impl TestProvider {
        fn new(results: impl IntoIterator<Item = Result<WeatherReport, io::Error>>) -> Self {
            Self(Mutex::new(results.into_iter().collect()))
        }
    }

    impl WeatherProvider for TestProvider {
        type Error = io::Error;

        async fn current(&self) -> Result<WeatherReport, io::Error> {
            self.0
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or_else(|| Ok(report()))
        }
    }

    #[derive(Default)]
    struct TestPublisher {
        failures_left: Mutex<u32>,
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
            let mut failures_left = self.failures_left.lock().expect("lock");
            if *failures_left > 0 {
                *failures_left -= 1;
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

    async fn run(provider: &TestProvider, publisher: &TestPublisher) {
        let source_id = SourceId::from("open-meteo".to_owned());
        let outcome = timeout(
            RUN_FOR,
            collect_weather(provider, publisher, &source_id, PERIOD),
        )
        .await;
        assert!(outcome.is_err(), "the loop never returns");
    }

    #[tokio::test(start_paused = true)]
    async fn publishes_every_period_starting_at_once() {
        let provider = TestProvider::new([]);
        let publisher = TestPublisher::default();
        run(&provider, &publisher).await;
        let published = publisher.published.lock().expect("lock");
        assert_eq!(published.len(), 3);
        for (kind, id, batch) in published.iter() {
            assert_eq!(*kind, SourceKind::Weather);
            assert_eq!(id.as_str(), "open-meteo");
            assert_eq!(batch.len(), 5);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn provider_error_publishes_nothing_and_the_loop_continues() {
        let provider = TestProvider::new([Err(io::Error::other("offline"))]);
        let publisher = TestPublisher::default();
        run(&provider, &publisher).await;
        assert_eq!(publisher.published.lock().expect("lock").len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn publish_error_is_skipped_and_the_next_tick_publishes() {
        let provider = TestProvider::new([]);
        let publisher = TestPublisher {
            failures_left: Mutex::new(1),
            ..TestPublisher::default()
        };
        run(&provider, &publisher).await;
        assert_eq!(publisher.published.lock().expect("lock").len(), 2);
    }
}
