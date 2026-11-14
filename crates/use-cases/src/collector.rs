use std::collections::{BTreeMap, HashMap};
use std::mem;
use std::time::Duration;

use bthome::{DecodeError, Packet};
use domain::{MacAddress, Measurement, Metric, SourceId, SourceKind};
use tokio::time::{Instant, interval_at};
use tracing::{debug, info, warn};

use crate::{Advertisement, AdvertisementSource, Publisher};

/// Keeps the newest value per sensor and metric and publishes it once per
/// flush period.
#[derive(Default)]
pub struct Collector {
    last_packet_id: HashMap<MacAddress, u8>,
    pending: BTreeMap<(SourceId, Metric), Measurement>,
}

impl Collector {
    /// Records the readings of a fresh BTHome advertisement.
    ///
    /// Only a changed packet id proves a fresh advertisement: BlueZ replays
    /// cached service data, so the first packet from a sensor just primes the
    /// id. Packets without an id cannot be checked and are ignored.
    pub fn observe(&mut self, advertisement: &Advertisement) -> Result<(), DecodeError> {
        let Some(data) = advertisement.service_data.get(&bthome::SERVICE_UUID) else {
            return Ok(());
        };
        let packet = Packet::parse(data)?;
        let Some(packet_id) = packet.packet_id else {
            return Ok(());
        };
        match self.last_packet_id.insert(advertisement.address, packet_id) {
            None => {
                info!(address = %advertisement.address, "first packet from sensor");
                return Ok(());
            }
            Some(previous) if previous == packet_id => return Ok(()),
            Some(_) => {}
        }
        let source_id = SourceId::from(advertisement.address);
        for reading in packet.readings {
            self.pending.insert(
                (source_id.clone(), reading.metric),
                Measurement {
                    timestamp: advertisement.received_at,
                    source_kind: SourceKind::Ble,
                    source_id: source_id.clone(),
                    metric: reading.metric,
                    value: reading.value,
                },
            );
        }
        Ok(())
    }

    /// Empties the pending measurements into one ordered batch per source.
    pub fn drain(&mut self) -> Vec<Vec<Measurement>> {
        let mut batches: Vec<Vec<Measurement>> = Vec::new();
        for measurement in mem::take(&mut self.pending).into_values() {
            match batches.last_mut() {
                Some(batch)
                    if batch
                        .last()
                        .is_some_and(|m| m.source_id == measurement.source_id) =>
                {
                    batch.push(measurement);
                }
                _ => batches.push(vec![measurement]),
            }
        }
        batches
    }

    /// Puts back an unpublished batch, keeping any value that arrived since.
    fn requeue(&mut self, batch: Vec<Measurement>) {
        for measurement in batch {
            self.pending
                .entry((measurement.source_id.clone(), measurement.metric))
                .or_insert(measurement);
        }
    }

    async fn flush<P: Publisher>(&mut self, publisher: &P) {
        for batch in self.drain() {
            let Some(first) = batch.first() else {
                continue;
            };
            let (source_kind, source_id) = (first.source_kind, first.source_id.clone());
            if let Err(error) = publisher.publish(source_kind, &source_id, &batch).await {
                warn!(source_id = source_id.as_str(), %error, "publish failed, keeping measurements for the next flush");
                self.requeue(batch);
            }
        }
    }

    /// Feeds advertisements into the collector and flushes every `period`.
    /// Returns when the source ends; publish errors never end the loop.
    pub async fn run<S: AdvertisementSource, P: Publisher>(
        mut self,
        mut source: S,
        publisher: &P,
        period: Duration,
    ) {
        let mut ticker = interval_at(Instant::now() + period, period);
        loop {
            tokio::select! {
                next = source.next_advertisement() => match next {
                    Some(advertisement) => {
                        if let Err(error) = self.observe(&advertisement) {
                            debug!(address = %advertisement.address, %error, "dropping undecodable advertisement");
                        }
                    }
                    None => return,
                },
                _ = ticker.tick() => self.flush(publisher).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::io;
    use std::sync::Mutex;
    use std::time::Duration;

    use domain::{MacAddress, Measurement, Metric, SourceId, SourceKind, Timestamp};
    use tokio::time::{Instant, sleep_until};

    use crate::{Advertisement, AdvertisementSource, Publisher};

    use super::Collector;

    const MAC_A: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 0xA]);
    const MAC_B: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 0xB]);

    fn advertisement(address: MacAddress, packet_id: u8, centi_celsius: i16) -> Advertisement {
        let [lo, hi] = centi_celsius.to_le_bytes();
        let payload = vec![0x40, 0x00, packet_id, 0x02, lo, hi, 0x01, 0x58];
        Advertisement {
            address,
            received_at: Timestamp::from_micros(i64::from(packet_id)),
            service_data: HashMap::from([(bthome::SERVICE_UUID, payload)]),
        }
    }

    fn measurement(address: MacAddress, metric: Metric, value: f64, micros: i64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: address.into(),
            metric,
            value,
        }
    }

    fn drained_values(collector: &mut Collector) -> Vec<(Metric, f64)> {
        collector
            .drain()
            .into_iter()
            .flatten()
            .map(|m| (m.metric, m.value))
            .collect()
    }

    #[test]
    fn first_packet_only_primes() {
        let mut collector = Collector::default();
        collector
            .observe(&advertisement(MAC_A, 1, 2000))
            .expect("decodes");
        assert!(collector.drain().is_empty());
    }

    #[test]
    fn last_value_per_metric_wins() {
        let mut collector = Collector::default();
        for (id, temperature) in [(1, 2000), (2, 2100), (3, 2200), (4, 2300)] {
            collector
                .observe(&advertisement(MAC_A, id, temperature))
                .expect("decodes");
        }
        assert_eq!(
            drained_values(&mut collector),
            [(Metric::Temperature, 23.0), (Metric::Battery, 88.0)]
        );
        assert!(collector.drain().is_empty());
    }

    #[test]
    fn repeated_packet_id_does_not_overwrite() {
        let mut collector = Collector::default();
        collector
            .observe(&advertisement(MAC_A, 1, 2000))
            .expect("decodes");
        collector
            .observe(&advertisement(MAC_A, 2, 2100))
            .expect("decodes");
        collector
            .observe(&advertisement(MAC_A, 2, 9900))
            .expect("decodes");
        assert_eq!(
            drained_values(&mut collector)[0],
            (Metric::Temperature, 21.0)
        );
    }

    #[test]
    fn each_sensor_gets_its_own_batch() {
        let mut collector = Collector::default();
        for address in [MAC_B, MAC_A] {
            collector
                .observe(&advertisement(address, 1, 2000))
                .expect("decodes");
            collector
                .observe(&advertisement(address, 2, 2100))
                .expect("decodes");
        }
        let batches = collector.drain();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0][0].source_id, SourceId::from(MAC_A));
        assert_eq!(batches[1][0].source_id, SourceId::from(MAC_B));
        assert!(batches.iter().all(|batch| batch.len() == 2));
    }

    #[test]
    fn other_services_are_ignored() {
        let mut collector = Collector::default();
        let mut other = advertisement(MAC_A, 1, 2000);
        other.service_data = HashMap::from([(0x181A, vec![1, 2, 3])]);
        collector.observe(&other).expect("ignored");
        assert!(collector.drain().is_empty());
    }

    #[test]
    fn packets_without_an_id_are_ignored() {
        let mut collector = Collector::default();
        let mut anonymous = advertisement(MAC_A, 1, 2000);
        anonymous.service_data = HashMap::from([(bthome::SERVICE_UUID, vec![0x40, 0x01, 0x58])]);
        collector.observe(&anonymous).expect("ignored");
        collector.observe(&anonymous).expect("ignored");
        assert!(collector.drain().is_empty());
    }

    #[test]
    fn undecodable_packets_are_an_error() {
        let mut collector = Collector::default();
        let mut encrypted = advertisement(MAC_A, 1, 2000);
        encrypted.service_data = HashMap::from([(bthome::SERVICE_UUID, vec![0x41])]);
        assert_eq!(
            collector.observe(&encrypted),
            Err(bthome::DecodeError::Encrypted)
        );
    }

    #[test]
    fn requeue_keeps_newer_values() {
        let mut collector = Collector::default();
        collector
            .observe(&advertisement(MAC_A, 1, 2000))
            .expect("decodes");
        collector
            .observe(&advertisement(MAC_A, 2, 2100))
            .expect("decodes");
        collector.requeue(vec![
            measurement(MAC_A, Metric::Temperature, 5.0, 1),
            measurement(MAC_A, Metric::Humidity, 50.0, 1),
        ]);
        let batch = collector.drain().remove(0);
        let values: Vec<_> = batch
            .iter()
            .map(|m| (m.metric, m.value, m.timestamp))
            .collect();
        assert_eq!(
            values,
            [
                (Metric::Temperature, 21.0, Timestamp::from_micros(2)),
                (Metric::Humidity, 50.0, Timestamp::from_micros(1)),
                (Metric::Battery, 88.0, Timestamp::from_micros(2)),
            ]
        );
    }

    struct VecSource(VecDeque<Advertisement>);

    impl AdvertisementSource for VecSource {
        async fn next_advertisement(&mut self) -> Option<Advertisement> {
            self.0.pop_front()
        }
    }

    /// Yields each advertisement at an offset from `start`; cancel-safe.
    struct TimedSource {
        start: Instant,
        queue: VecDeque<(Duration, Advertisement)>,
    }

    impl AdvertisementSource for TimedSource {
        async fn next_advertisement(&mut self) -> Option<Advertisement> {
            let (offset, _) = self.queue.front()?;
            sleep_until(self.start + *offset).await;
            self.queue
                .pop_front()
                .map(|(_, advertisement)| advertisement)
        }
    }

    #[derive(Default)]
    struct TestPublisher {
        failures_left: Mutex<u32>,
        published: Mutex<Vec<Vec<Measurement>>>,
    }

    impl TestPublisher {
        fn failing(times: u32) -> Self {
            Self {
                failures_left: Mutex::new(times),
                ..Self::default()
            }
        }

        fn published(&self) -> Vec<Vec<Measurement>> {
            self.published.lock().expect("lock").clone()
        }
    }

    impl Publisher for TestPublisher {
        type Error = io::Error;

        async fn publish(
            &self,
            _source_kind: SourceKind,
            _source_id: &SourceId,
            measurements: &[Measurement],
        ) -> Result<(), io::Error> {
            let mut failures_left = self.failures_left.lock().expect("lock");
            if *failures_left > 0 {
                *failures_left -= 1;
                return Err(io::Error::other("broker down"));
            }
            self.published
                .lock()
                .expect("lock")
                .push(measurements.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    async fn failed_publish_is_retried_on_the_next_flush() {
        let mut collector = Collector::default();
        collector
            .observe(&advertisement(MAC_A, 1, 2000))
            .expect("decodes");
        collector
            .observe(&advertisement(MAC_A, 2, 2100))
            .expect("decodes");
        let publisher = TestPublisher::failing(1);

        collector.flush(&publisher).await;
        assert!(publisher.published().is_empty());

        collector
            .observe(&advertisement(MAC_A, 3, 2200))
            .expect("decodes");
        collector.flush(&publisher).await;
        let published = publisher.published();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0][0].value, 22.0);

        collector.flush(&publisher).await;
        assert_eq!(publisher.published().len(), 1);
    }

    #[tokio::test]
    async fn failed_publish_with_no_newer_value_is_republished() {
        let mut collector = Collector::default();
        collector
            .observe(&advertisement(MAC_A, 1, 2000))
            .expect("decodes");
        collector
            .observe(&advertisement(MAC_A, 2, 2100))
            .expect("decodes");
        let publisher = TestPublisher::failing(1);

        collector.flush(&publisher).await;
        collector.flush(&publisher).await;
        let published = publisher.published();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0][0].value, 21.0);
        assert_eq!(published[0][0].timestamp, Timestamp::from_micros(2));
    }

    #[tokio::test]
    async fn run_returns_when_the_source_ends() {
        let publisher = TestPublisher::default();
        Collector::default()
            .run(
                VecSource(VecDeque::new()),
                &publisher,
                Duration::from_secs(60),
            )
            .await;
        assert!(publisher.published().is_empty());
    }

    async fn published_temperatures(publish_failures: u32) -> Vec<f64> {
        let at = |secs| Duration::from_secs(secs);
        let source = TimedSource {
            start: Instant::now(),
            queue: VecDeque::from([
                (at(0), advertisement(MAC_A, 1, 2000)),
                (at(10), advertisement(MAC_A, 2, 2100)),
                (at(20), advertisement(MAC_A, 3, 2200)),
                (at(70), advertisement(MAC_A, 4, 2300)),
                (at(130), advertisement(MAC_A, 4, 2300)),
            ]),
        };
        let publisher = TestPublisher::failing(publish_failures);
        Collector::default().run(source, &publisher, at(60)).await;
        publisher.published().iter().map(|b| b[0].value).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn run_publishes_once_per_period() {
        assert_eq!(published_temperatures(0).await, [22.0, 23.0]);
    }

    #[tokio::test(start_paused = true)]
    async fn run_survives_publish_errors() {
        assert_eq!(published_temperatures(1).await, [23.0]);
    }
}
