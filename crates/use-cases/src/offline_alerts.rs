use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

use domain::{Measurement, OfflineEvent, OfflineWatch, RoomName, SourceId, Thermometer, Timestamp};
use tracing::{info, warn};

use crate::{Notification, Notifier};

struct Watched {
    room: RoomName,
    watch: OfflineWatch,
}

/// Watches every configured sensor and sends the notifications they are owed.
pub struct OfflineAlerts {
    // A borrow is never held across an `.await`, so measurements can be
    // observed while a notification is being sent.
    sensors: RefCell<BTreeMap<SourceId, Watched>>,
}

impl OfflineAlerts {
    /// Every thermometer counts as last seen at `now`.
    pub fn new(thermometers: &[Thermometer], now: Timestamp) -> Self {
        let sensors = thermometers
            .iter()
            .map(|thermometer| {
                let watched = Watched {
                    room: thermometer.room.clone(),
                    watch: OfflineWatch::new(now),
                };
                (SourceId::from(thermometer.mac), watched)
            })
            .collect();
        Self {
            sensors: RefCell::new(sensors),
        }
    }

    /// Records measurement times of configured sensors; others are ignored.
    pub fn observe(&self, measurements: &[Measurement]) {
        let mut sensors = self.sensors.borrow_mut();
        for measurement in measurements {
            if let Some(watched) = sensors.get_mut(&measurement.source_id) {
                watched.watch.saw(measurement.timestamp);
            }
        }
    }

    /// Sends the notification each sensor is owed. Each decision is made from
    /// the state just before its send and recorded at the time of delivery;
    /// a failed send leaves the state unchanged, so the next call retries.
    pub async fn notify_due<N: Notifier, C: Fn() -> Timestamp>(&self, notifier: &N, now: C) {
        let source_ids: Vec<SourceId> = self.sensors.borrow().keys().cloned().collect();
        for source_id in source_ids {
            let Some((event, notification)) = self.pending(&source_id, now()) else {
                continue;
            };
            match notifier.notify(&notification).await {
                Ok(()) => self.delivered(&source_id, event, now()),
                Err(error) => {
                    warn!(source_id = source_id.as_str(), %error, "notification failed, retrying at the next check");
                }
            }
        }
    }

    fn pending(
        &self,
        source_id: &SourceId,
        now: Timestamp,
    ) -> Option<(OfflineEvent, Notification)> {
        let sensors = self.sensors.borrow();
        let watched = sensors.get(source_id)?;
        let event = watched.watch.due(now)?;
        Some((event, notification(&watched.room, event)))
    }

    fn delivered(&self, source_id: &SourceId, event: OfflineEvent, now: Timestamp) {
        if let Some(watched) = self.sensors.borrow_mut().get_mut(source_id) {
            watched.watch.delivered(event, now);
            info!(room = %watched.room, ?event, "notification delivered");
        }
    }
}

fn notification(room: &RoomName, event: OfflineEvent) -> Notification {
    let (title, message) = match event {
        OfflineEvent::Offline { silent_for } => (
            format!("{room} thermometer offline"),
            silent_message(silent_for),
        ),
        OfflineEvent::StillOffline { silent_for } => (
            format!("{room} thermometer still offline"),
            silent_message(silent_for),
        ),
        OfflineEvent::Back => (
            format!("{room} thermometer is back"),
            "Reporting again.".to_owned(),
        ),
    };
    Notification { title, message }
}

fn silent_message(silent_for: Duration) -> String {
    format!("No reading for {} h.", silent_for.as_secs() / 3600)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::future::{self, Future};
    use std::io;
    use std::time::Duration;

    use domain::{MacAddress, Measurement, Metric, RoomName, SourceKind, Thermometer, Timestamp};

    use crate::{Notification, Notifier};

    use super::OfflineAlerts;

    const MAC_BEDROOM: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 1]);
    const MAC_OFFICE: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 2]);
    const MAC_STRANGER: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9]);
    const START: i64 = 1_000_000_000_000;
    const HOUR: i64 = 3600;

    struct FakeNotifier<H: Fn()> {
        failures_left: Cell<u32>,
        sent: RefCell<Vec<Notification>>,
        on_send: H,
    }

    impl FakeNotifier<fn()> {
        fn new() -> Self {
            Self::with_hook(|| {})
        }

        fn failing(times: u32) -> Self {
            let notifier = Self::new();
            notifier.failures_left.set(times);
            notifier
        }
    }

    impl<H: Fn()> FakeNotifier<H> {
        fn with_hook(on_send: H) -> Self {
            Self {
                failures_left: Cell::new(0),
                sent: RefCell::new(Vec::new()),
                on_send,
            }
        }

        fn titles(&self) -> Vec<String> {
            self.sent.borrow().iter().map(|n| n.title.clone()).collect()
        }

        fn messages(&self) -> Vec<String> {
            self.sent
                .borrow()
                .iter()
                .map(|n| n.message.clone())
                .collect()
        }
    }

    impl<H: Fn()> Notifier for FakeNotifier<H> {
        type Error = io::Error;

        fn notify(
            &self,
            notification: &Notification,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send {
            (self.on_send)();
            let outcome = if self.failures_left.get() > 0 {
                self.failures_left.set(self.failures_left.get() - 1);
                Err(io::Error::other("ntfy is down"))
            } else {
                self.sent.borrow_mut().push(notification.clone());
                Ok(())
            };
            future::ready(outcome)
        }
    }

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_micros(START + secs * 1_000_000)
    }

    fn thermometers() -> Vec<Thermometer> {
        [(MAC_BEDROOM, "bedroom"), (MAC_OFFICE, "office")]
            .into_iter()
            .map(|(mac, room)| Thermometer {
                mac,
                room: RoomName::try_from(room.to_owned()).expect("valid room"),
            })
            .collect()
    }

    fn alerts() -> OfflineAlerts {
        OfflineAlerts::new(&thermometers(), at(0))
    }

    fn reading(mac: MacAddress, secs: i64) -> Measurement {
        Measurement {
            timestamp: at(secs),
            source_kind: SourceKind::Ble,
            source_id: mac.into(),
            metric: Metric::Temperature,
            value: 21.0,
        }
    }

    async fn check<H: Fn()>(alerts: &OfflineAlerts, notifier: &FakeNotifier<H>, secs: i64) {
        alerts.notify_due(notifier, || at(secs)).await;
    }

    #[tokio::test]
    async fn sensor_lifecycle() {
        let alerts = alerts();
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, HOUR - 60).await;
        assert!(notifier.sent.borrow().is_empty());
        check(&alerts, &notifier, HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
        assert_eq!(notifier.messages(), ["No reading for 1 h."]);
        check(&alerts, &notifier, HOUR + 60).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, 13 * HOUR).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer still offline");
        assert_eq!(notifier.messages()[1], "No reading for 13 h.");
        check(&alerts, &notifier, 25 * HOUR).await;
        assert_eq!(notifier.messages()[2], "No reading for 25 h.");
        alerts.observe(&[reading(MAC_BEDROOM, 25 * HOUR + 10)]);
        check(&alerts, &notifier, 25 * HOUR + 20).await;
        assert_eq!(notifier.titles()[3], "bedroom thermometer is back");
        assert_eq!(notifier.messages()[3], "Reporting again.");
        check(&alerts, &notifier, 25 * HOUR + 80).await;
        assert_eq!(notifier.sent.borrow().len(), 4);
    }

    #[tokio::test]
    async fn restart_counts_every_sensor_as_seen_at_startup() {
        let restarted = OfflineAlerts::new(&thermometers(), at(100 * HOUR));
        let notifier = FakeNotifier::new();
        check(&restarted, &notifier, 101 * HOUR - 1).await;
        assert!(notifier.sent.borrow().is_empty());
        check(&restarted, &notifier, 101 * HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn backlog_with_old_timestamps_changes_nothing() {
        let alerts = alerts();
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_BEDROOM, -50 * HOUR)]);
        check(&alerts, &notifier, HOUR).await;
        assert_eq!(
            notifier.titles(),
            ["bedroom thermometer offline", "office thermometer offline"]
        );
        alerts.observe(&[reading(MAC_BEDROOM, -40 * HOUR)]);
        check(&alerts, &notifier, HOUR + 60).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn failed_send_is_retried_and_recorded_once() {
        let alerts = alerts();
        alerts.observe(&[reading(MAC_OFFICE, HOUR)]);
        let notifier = FakeNotifier::failing(1);
        check(&alerts, &notifier, HOUR).await;
        assert!(notifier.sent.borrow().is_empty());
        check(&alerts, &notifier, HOUR + 60).await;
        check(&alerts, &notifier, HOUR + 120).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
    }

    #[tokio::test]
    async fn failed_back_is_retried() {
        let alerts = alerts();
        alerts.observe(&[reading(MAC_OFFICE, HOUR)]);
        let notifier = FakeNotifier::new();
        check(&alerts, &notifier, HOUR).await;
        alerts.observe(&[reading(MAC_BEDROOM, HOUR + 10)]);
        notifier.failures_left.set(1);
        check(&alerts, &notifier, HOUR + 20).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, HOUR + 80).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer is back");
    }

    #[tokio::test]
    async fn report_after_a_failed_offline_send_sends_nothing() {
        let alerts = alerts();
        let notifier = FakeNotifier::failing(2);
        check(&alerts, &notifier, HOUR).await;
        alerts.observe(&[
            reading(MAC_BEDROOM, HOUR + 10),
            reading(MAC_OFFICE, HOUR + 10),
        ]);
        check(&alerts, &notifier, HOUR + 60).await;
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn unknown_sensor_never_notifies() {
        let alerts = alerts();
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_STRANGER, 10)]);
        alerts.observe(&[reading(MAC_BEDROOM, HOUR), reading(MAC_OFFICE, HOUR)]);
        check(&alerts, &notifier, 100 * HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn only_the_silent_room_is_named() {
        let alerts = alerts();
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, HOUR - 10)]);
        check(&alerts, &notifier, HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
    }

    #[tokio::test]
    async fn recovery_during_an_earlier_send_cancels_the_stale_offline() {
        let alerts = alerts();
        let notifier = FakeNotifier::with_hook(|| alerts.observe(&[reading(MAC_OFFICE, HOUR)]));
        check(&alerts, &notifier, HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
    }

    #[tokio::test]
    async fn reminder_counts_from_delivery_completion() {
        let alerts = alerts();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        let clock = Cell::new(HOUR);
        let notifier = FakeNotifier::with_hook(|| clock.set(HOUR + 1800));
        alerts.notify_due(&notifier, || at(clock.get())).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, 13 * HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, 13 * HOUR + 1800).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer still offline");
    }

    #[test]
    fn silent_message_rounds_down_to_whole_hours() {
        assert_eq!(
            super::silent_message(Duration::from_secs(2 * 3600 + 3599)),
            "No reading for 2 h."
        );
    }
}
