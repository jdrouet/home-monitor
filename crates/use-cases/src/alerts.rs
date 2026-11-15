use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use domain::{
    AlertState, Measurement, Metric, OfflineEvent, OfflineWatch, RoomName, SourceId,
    TemperatureEvent, TemperatureWatch, Thermometer, Timestamp,
};
use tracing::{error, info, warn};

use crate::offline_notification::offline_notification;
use crate::temperature_notification::temperature_notification;
use crate::{AlertStateStore, Notification, Notifier};

#[derive(Clone, Copy)]
enum Rule {
    Offline,
    Temperature,
}

const RULES: [Rule; 2] = [Rule::Offline, Rule::Temperature];

#[derive(Debug, Clone, Copy)]
enum Event {
    Offline(OfflineEvent),
    Temperature(TemperatureEvent),
}

struct Watched {
    room: RoomName,
    offline: OfflineWatch,
    temperature: Option<TemperatureWatch>,
}

impl Watched {
    fn pending(&self, rule: Rule, now: Timestamp) -> Option<(Event, Notification)> {
        match rule {
            Rule::Offline => {
                let event = self.offline.due(now)?;
                Some((
                    Event::Offline(event),
                    offline_notification(&self.room, event),
                ))
            }
            Rule::Temperature => {
                let watch = self.temperature.as_ref()?;
                let event = watch.due(now)?;
                Some((
                    Event::Temperature(event),
                    temperature_notification(&self.room, event, watch.range()),
                ))
            }
        }
    }

    fn delivered(&mut self, event: Event, now: Timestamp) {
        match (event, self.temperature.as_mut()) {
            (Event::Offline(event), _) => self.offline.delivered(event, now),
            (Event::Temperature(event), Some(watch)) => watch.delivered(event, now),
            (Event::Temperature(_), None) => {}
        }
    }
}

/// Watches every configured sensor and sends the notifications they are owed.
pub struct Alerts {
    // A borrow is never held across an `.await`, so measurements can be
    // observed while a notification is being sent.
    sensors: RefCell<BTreeMap<SourceId, Watched>>,
    // Set when the last save failed; the next check saves again.
    unsaved: Cell<bool>,
}

impl Alerts {
    /// Resumes the alerts the store holds. Sensors with none count as last
    /// seen at `now`; an unreadable store starts every sensor that way.
    pub fn new<S: AlertStateStore>(
        thermometers: &[Thermometer],
        store: &S,
        now: Timestamp,
    ) -> Self {
        let state = store.load().unwrap_or_else(|error| {
            error!(%error, "cannot load the alert state, starting without it");
            AlertState::default()
        });
        let sensors = thermometers
            .iter()
            .map(|thermometer| {
                let source_id = SourceId::from(thermometer.mac);
                let offline = state.offline.get(&source_id).map_or_else(
                    || OfflineWatch::new(now),
                    |alert| OfflineWatch::from(*alert),
                );
                let temperature = thermometer.temperature_range.map(|range| {
                    TemperatureWatch::new(range, state.temperature.get(&source_id).copied())
                });
                let watched = Watched {
                    room: thermometer.room.clone(),
                    offline,
                    temperature,
                };
                (source_id, watched)
            })
            .collect();
        Self {
            sensors: RefCell::new(sensors),
            unsaved: Cell::new(false),
        }
    }

    /// Records measurements of configured sensors; others are ignored. Every
    /// metric shows a sensor is alive; only temperatures feed the range rule.
    pub fn observe(&self, measurements: &[Measurement]) {
        let mut sensors = self.sensors.borrow_mut();
        for measurement in measurements {
            let Some(watched) = sensors.get_mut(&measurement.source_id) else {
                continue;
            };
            watched.offline.saw(measurement.timestamp);
            if let (Metric::Temperature, Some(watch)) =
                (measurement.metric, watched.temperature.as_mut())
            {
                watch.saw(measurement.timestamp, measurement.value);
            }
        }
    }

    /// Sends the notification each sensor is owed. Each decision is made from
    /// the state just before its send and recorded at the time of delivery,
    /// then saved; a failed send leaves the state unchanged, so the next call
    /// retries, and a failed save is retried at the start of the next call.
    pub async fn notify_due<N: Notifier, S: AlertStateStore, C: Fn() -> Timestamp>(
        &self,
        notifier: &N,
        store: &S,
        now: C,
    ) {
        if self.unsaved.get() {
            self.save(store);
        }
        let source_ids: Vec<SourceId> = self.sensors.borrow().keys().cloned().collect();
        for source_id in source_ids {
            for rule in RULES {
                let Some((event, notification)) = self.pending(&source_id, rule, now()) else {
                    continue;
                };
                match notifier.notify(&notification).await {
                    Ok(()) => {
                        self.delivered(&source_id, event, now());
                        self.save(store);
                    }
                    Err(error) => {
                        warn!(source_id = source_id.as_str(), %error, "notification failed, retrying at the next check");
                    }
                }
            }
        }
    }

    fn pending(
        &self,
        source_id: &SourceId,
        rule: Rule,
        now: Timestamp,
    ) -> Option<(Event, Notification)> {
        self.sensors.borrow().get(source_id)?.pending(rule, now)
    }

    fn delivered(&self, source_id: &SourceId, event: Event, now: Timestamp) {
        if let Some(watched) = self.sensors.borrow_mut().get_mut(source_id) {
            watched.delivered(event, now);
            info!(room = %watched.room, ?event, "notification delivered");
        }
    }

    fn snapshot(&self) -> AlertState {
        let sensors = self.sensors.borrow();
        let offline = sensors
            .iter()
            .filter_map(|(source_id, watched)| Some((source_id.clone(), watched.offline.active()?)))
            .collect();
        let temperature = sensors
            .iter()
            .filter_map(|(source_id, watched)| {
                Some((source_id.clone(), watched.temperature.as_ref()?.active()?))
            })
            .collect();
        AlertState {
            offline,
            temperature,
        }
    }

    fn save<S: AlertStateStore>(&self, store: &S) {
        match store.save(&self.snapshot()) {
            Ok(()) => self.unsaved.set(false),
            Err(error) => {
                warn!(%error, "cannot save the alert state, retrying at the next check");
                self.unsaved.set(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::future::{self, Future};
    use std::io;

    use domain::{
        AlertState, Deviation, MacAddress, Measurement, Metric, OfflineAlert, RoomName, SourceKind,
        TemperatureAlert, TemperatureRange, Thermometer, Timestamp,
    };

    use crate::{AlertStateStore, Notification, Notifier};

    use super::Alerts;

    const MAC_BEDROOM: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 1]);
    const MAC_OFFICE: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 2]);
    const MAC_STRANGER: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9]);
    const START: i64 = 1_000_000_000_000;
    const MINUTE: i64 = 60;
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

    struct FakeStore {
        loaded: Option<AlertState>,
        saved: RefCell<Vec<AlertState>>,
        failures_left: Cell<u32>,
    }

    impl FakeStore {
        fn new() -> Self {
            Self::holding(AlertState::default())
        }

        fn holding(state: AlertState) -> Self {
            Self {
                loaded: Some(state),
                saved: RefCell::new(Vec::new()),
                failures_left: Cell::new(0),
            }
        }

        fn unreadable() -> Self {
            Self {
                loaded: None,
                ..Self::new()
            }
        }
    }

    impl AlertStateStore for FakeStore {
        type Error = io::Error;

        fn load(&self) -> Result<AlertState, Self::Error> {
            self.loaded
                .clone()
                .ok_or_else(|| io::Error::other("state file is corrupt"))
        }

        fn save(&self, state: &AlertState) -> Result<(), Self::Error> {
            if self.failures_left.get() > 0 {
                self.failures_left.set(self.failures_left.get() - 1);
                return Err(io::Error::other("disk is full"));
            }
            self.saved.borrow_mut().push(state.clone());
            Ok(())
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
                temperature_range: None,
            })
            .collect()
    }

    fn alerts(store: &FakeStore, now: i64) -> Alerts {
        Alerts::new(&thermometers(), store, at(now))
    }

    fn measured(mac: MacAddress, secs: i64, metric: Metric, value: f64) -> Measurement {
        Measurement {
            timestamp: at(secs),
            source_kind: SourceKind::Ble,
            source_id: mac.into(),
            metric,
            value,
        }
    }

    fn reading(mac: MacAddress, secs: i64) -> Measurement {
        measured(mac, secs, Metric::Temperature, 21.0)
    }

    // The bedroom must stay within 18 to 20 °C; the office has no range.
    fn ranged_alerts(store: &FakeStore, now: i64) -> Alerts {
        let mut thermometers = thermometers();
        thermometers[0].temperature_range =
            Some(TemperatureRange::new(18.0, 20.0).expect("valid range"));
        Alerts::new(&thermometers, store, at(now))
    }

    // Both sensors report at `secs`, so neither counts as silent.
    fn live(alerts: &Alerts, secs: i64, bedroom_celsius: f64) {
        alerts.observe(&[
            measured(MAC_BEDROOM, secs, Metric::Temperature, bedroom_celsius),
            reading(MAC_OFFICE, secs),
        ]);
    }

    fn hot_alert(notified_at: i64) -> FakeStore {
        let mut state = AlertState::default();
        state.temperature.insert(
            MAC_BEDROOM.into(),
            TemperatureAlert {
                deviation: Deviation::TooHot,
                celsius: 21.3,
                since: at(notified_at),
                notified_at: at(notified_at),
            },
        );
        FakeStore::holding(state)
    }

    // Leaves the bedroom with a delivered "too hot" alert at 15 minutes.
    async fn too_hot<H: Fn()>(alerts: &Alerts, notifier: &FakeNotifier<H>, store: &FakeStore) {
        live(alerts, 0, 21.3);
        live(alerts, 15 * MINUTE, 21.3);
        check(alerts, notifier, store, 15 * MINUTE).await;
    }

    async fn check<H: Fn()>(
        alerts: &Alerts,
        notifier: &FakeNotifier<H>,
        store: &FakeStore,
        secs: i64,
    ) {
        alerts.notify_due(notifier, store, || at(secs)).await;
    }

    fn persisted_bedroom_alert() -> FakeStore {
        let mut state = AlertState::default();
        state.offline.insert(
            MAC_BEDROOM.into(),
            OfflineAlert {
                last_seen: at(0),
                since: at(HOUR),
                notified_at: at(HOUR),
            },
        );
        FakeStore::holding(state)
    }

    #[tokio::test]
    async fn sensor_lifecycle() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR - 60).await;
        assert!(notifier.sent.borrow().is_empty());
        check(&alerts, &notifier, &store, HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
        assert_eq!(notifier.messages(), ["No reading for 1 h."]);
        check(&alerts, &notifier, &store, HOUR + 60).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, &store, 13 * HOUR).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer still offline");
        assert_eq!(notifier.messages()[1], "No reading for 13 h.");
        check(&alerts, &notifier, &store, 25 * HOUR).await;
        assert_eq!(notifier.messages()[2], "No reading for 25 h.");
        alerts.observe(&[reading(MAC_BEDROOM, 25 * HOUR + 10)]);
        check(&alerts, &notifier, &store, 25 * HOUR + 20).await;
        assert_eq!(notifier.titles()[3], "bedroom thermometer is back");
        assert_eq!(notifier.messages()[3], "Reporting again.");
        check(&alerts, &notifier, &store, 25 * HOUR + 80).await;
        assert_eq!(notifier.sent.borrow().len(), 4);
    }

    #[tokio::test]
    async fn restart_counts_every_sensor_as_seen_at_startup() {
        let store = FakeStore::new();
        let restarted = alerts(&store, 100 * HOUR);
        let notifier = FakeNotifier::new();
        check(&restarted, &notifier, &store, 101 * HOUR - 1).await;
        assert!(notifier.sent.borrow().is_empty());
        check(&restarted, &notifier, &store, 101 * HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn backlog_with_old_timestamps_changes_nothing() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_BEDROOM, -50 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR).await;
        assert_eq!(
            notifier.titles(),
            ["bedroom thermometer offline", "office thermometer offline"]
        );
        alerts.observe(&[reading(MAC_BEDROOM, -40 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR + 60).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn failed_send_is_retried_and_recorded_once() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        alerts.observe(&[reading(MAC_OFFICE, HOUR)]);
        let notifier = FakeNotifier::failing(1);
        check(&alerts, &notifier, &store, HOUR).await;
        assert!(notifier.sent.borrow().is_empty());
        assert!(store.saved.borrow().is_empty());
        check(&alerts, &notifier, &store, HOUR + 60).await;
        check(&alerts, &notifier, &store, HOUR + 120).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
        assert_eq!(store.saved.borrow().len(), 1);
    }

    #[tokio::test]
    async fn failed_back_is_retried() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        alerts.observe(&[reading(MAC_OFFICE, HOUR)]);
        let notifier = FakeNotifier::new();
        check(&alerts, &notifier, &store, HOUR).await;
        alerts.observe(&[reading(MAC_BEDROOM, HOUR + 10)]);
        notifier.failures_left.set(1);
        check(&alerts, &notifier, &store, HOUR + 20).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, &store, HOUR + 80).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer is back");
    }

    #[tokio::test]
    async fn report_after_a_failed_offline_send_sends_nothing() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::failing(2);
        check(&alerts, &notifier, &store, HOUR).await;
        alerts.observe(&[
            reading(MAC_BEDROOM, HOUR + 10),
            reading(MAC_OFFICE, HOUR + 10),
        ]);
        check(&alerts, &notifier, &store, HOUR + 60).await;
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn unknown_sensor_never_notifies() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_STRANGER, 10)]);
        alerts.observe(&[reading(MAC_BEDROOM, HOUR), reading(MAC_OFFICE, HOUR)]);
        check(&alerts, &notifier, &store, 100 * HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn only_the_silent_room_is_named() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, HOUR - 10)]);
        check(&alerts, &notifier, &store, HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
    }

    #[tokio::test]
    async fn recovery_during_an_earlier_send_cancels_the_stale_offline() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::with_hook(|| alerts.observe(&[reading(MAC_OFFICE, HOUR)]));
        check(&alerts, &notifier, &store, HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer offline"]);
    }

    #[tokio::test]
    async fn reminder_counts_from_delivery_completion() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        let clock = Cell::new(HOUR);
        let notifier = FakeNotifier::with_hook(|| clock.set(HOUR + 1800));
        alerts
            .notify_due(&notifier, &store, || at(clock.get()))
            .await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, &store, 13 * HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, &store, 13 * HOUR + 1800).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer still offline");
    }

    #[tokio::test]
    async fn every_delivery_is_saved() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR).await;
        alerts.observe(&[reading(MAC_BEDROOM, HOUR + 10)]);
        check(&alerts, &notifier, &store, HOUR + 20).await;
        let saved = store.saved.borrow();
        assert_eq!(saved.len(), 2);
        let alert = saved[0].offline[&MAC_BEDROOM.into()];
        assert_eq!(alert.notified_at, at(HOUR));
        assert_eq!(alert.last_seen, at(0));
        assert!(saved[1].offline.is_empty());
    }

    #[tokio::test]
    async fn a_restart_keeps_the_persisted_alert() {
        let store = persisted_bedroom_alert();
        let restarted = alerts(&store, 2 * HOUR);
        let notifier = FakeNotifier::new();
        check(&restarted, &notifier, &store, 2 * HOUR).await;
        assert!(notifier.sent.borrow().is_empty());
        restarted.observe(&[reading(MAC_OFFICE, 13 * HOUR - 10)]);
        check(&restarted, &notifier, &store, 13 * HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom thermometer still offline"]);
        assert_eq!(notifier.messages(), ["No reading for 13 h."]);
        restarted.observe(&[reading(MAC_BEDROOM, 13 * HOUR + 10)]);
        check(&restarted, &notifier, &store, 13 * HOUR + 20).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer is back");
        assert!(
            store
                .saved
                .borrow()
                .last()
                .expect("saved")
                .offline
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_failed_save_is_not_resent_and_is_retried() {
        let store = FakeStore::new();
        store.failures_left.set(1);
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        assert!(alerts.unsaved.get());
        assert!(store.saved.borrow().is_empty());
        check(&alerts, &notifier, &store, HOUR + 60).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        assert!(!alerts.unsaved.get());
        assert_eq!(store.saved.borrow().len(), 1);
    }

    #[tokio::test]
    async fn an_unreadable_store_starts_from_the_baseline() {
        let store = FakeStore::unreadable();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        check(&alerts, &notifier, &store, HOUR - 1).await;
        assert!(notifier.sent.borrow().is_empty());
        check(&alerts, &notifier, &store, HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn persisted_alerts_of_unconfigured_sensors_are_dropped() {
        let mut state = AlertState::default();
        state.offline.insert(
            MAC_STRANGER.into(),
            OfflineAlert {
                last_seen: at(0),
                since: at(HOUR),
                notified_at: at(HOUR),
            },
        );
        let store = FakeStore::holding(state);
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR).await;
        let saved = store.saved.borrow();
        let keys: Vec<_> = saved[0].offline.keys().cloned().collect();
        assert_eq!(keys, [MAC_BEDROOM.into()]);
    }

    #[tokio::test]
    async fn too_hot_is_reported_after_fifteen_minutes() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        live(&alerts, 0, 21.3);
        live(&alerts, 15 * MINUTE - 1, 21.3);
        check(&alerts, &notifier, &store, 15 * MINUTE - 1).await;
        assert!(notifier.sent.borrow().is_empty());
        live(&alerts, 15 * MINUTE, 21.3);
        check(&alerts, &notifier, &store, 15 * MINUTE).await;
        assert_eq!(notifier.titles(), ["bedroom too hot"]);
        assert_eq!(notifier.messages(), ["21.3 °C, allowed 18 to 20 °C."]);
        let saved = store.saved.borrow();
        let alert = saved[0].temperature[&MAC_BEDROOM.into()];
        assert_eq!(alert.deviation, Deviation::TooHot);
        assert_eq!(alert.notified_at, at(15 * MINUTE));
    }

    #[tokio::test]
    async fn a_reading_inside_restarts_the_hold() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        live(&alerts, 0, 21.3);
        live(&alerts, 10 * MINUTE, 21.3);
        live(&alerts, 11 * MINUTE, 19.0);
        live(&alerts, 12 * MINUTE, 21.3);
        live(&alerts, 26 * MINUTE, 21.3);
        check(&alerts, &notifier, &store, 26 * MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
        live(&alerts, 27 * MINUTE, 21.3);
        check(&alerts, &notifier, &store, 27 * MINUTE).await;
        assert_eq!(notifier.titles(), ["bedroom too hot"]);
    }

    #[tokio::test]
    async fn hovering_at_the_limit_does_not_repeat() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        too_hot(&alerts, &notifier, &store).await;
        for step in 4..=28 {
            let celsius = if step % 2 == 0 { 21.3 } else { 19.0 };
            live(&alerts, step * 5 * MINUTE, celsius);
            check(&alerts, &notifier, &store, step * 5 * MINUTE).await;
        }
        assert_eq!(notifier.titles(), ["bedroom too hot"]);
    }

    #[tokio::test]
    async fn reminder_counts_from_the_delivery() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        live(&alerts, 0, 21.3);
        live(&alerts, 15 * MINUTE, 21.3);
        let clock = Cell::new(15 * MINUTE);
        let notifier = FakeNotifier::with_hook(|| clock.set(16 * MINUTE));
        alerts
            .notify_due(&notifier, &store, || at(clock.get()))
            .await;
        live(&alerts, 12 * HOUR + 15 * MINUTE, 22.0);
        check(&alerts, &notifier, &store, 12 * HOUR + 15 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check(&alerts, &notifier, &store, 12 * HOUR + 16 * MINUTE).await;
        assert_eq!(notifier.titles()[1], "bedroom still too hot");
        assert_eq!(notifier.messages()[1], "22.0 °C, allowed 18 to 20 °C.");
    }

    #[tokio::test]
    async fn resolved_after_fifteen_minutes_inside() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        too_hot(&alerts, &notifier, &store).await;
        live(&alerts, 20 * MINUTE, 19.0);
        live(&alerts, 34 * MINUTE, 19.0);
        check(&alerts, &notifier, &store, 34 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        live(&alerts, 35 * MINUTE, 19.0);
        check(&alerts, &notifier, &store, 35 * MINUTE).await;
        assert_eq!(notifier.titles()[1], "bedroom temperature back in range");
        assert_eq!(notifier.messages()[1], "19.0 °C, allowed 18 to 20 °C.");
        assert!(
            store
                .saved
                .borrow()
                .last()
                .expect("saved")
                .temperature
                .is_empty()
        );
        check(&alerts, &notifier, &store, 36 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn a_restart_keeps_the_persisted_temperature_alert() {
        let store = hot_alert(0);
        let restarted = ranged_alerts(&store, 2 * HOUR);
        let notifier = FakeNotifier::new();
        live(&restarted, 2 * HOUR, 21.3);
        live(&restarted, 2 * HOUR + 15 * MINUTE, 21.3);
        check(&restarted, &notifier, &store, 2 * HOUR + 15 * MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
        live(&restarted, 12 * HOUR, 21.5);
        check(&restarted, &notifier, &store, 12 * HOUR).await;
        assert_eq!(notifier.titles(), ["bedroom still too hot"]);
        live(&restarted, 12 * HOUR + 10 * MINUTE, 19.0);
        live(&restarted, 12 * HOUR + 25 * MINUTE, 19.0);
        check(&restarted, &notifier, &store, 12 * HOUR + 25 * MINUTE).await;
        assert_eq!(notifier.titles()[1], "bedroom temperature back in range");
    }

    #[tokio::test]
    async fn without_readings_a_reminder_names_the_last_temperature() {
        let store = hot_alert(0);
        let restarted = ranged_alerts(&store, 11 * HOUR);
        let notifier = FakeNotifier::new();
        restarted.observe(&[reading(MAC_OFFICE, 12 * HOUR)]);
        check(&restarted, &notifier, &store, 12 * HOUR).await;
        assert_eq!(
            notifier.titles(),
            ["bedroom thermometer offline", "bedroom still too hot"]
        );
        assert_eq!(notifier.messages()[1], "21.3 °C, allowed 18 to 20 °C.");
    }

    #[tokio::test]
    async fn a_room_without_a_range_never_notifies() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        for step in 0..=11 {
            let secs = step * 5 * MINUTE;
            alerts.observe(&[
                measured(MAC_BEDROOM, secs, Metric::Temperature, 19.0),
                measured(MAC_OFFICE, secs, Metric::Temperature, 40.0),
            ]);
            check(&alerts, &notifier, &store, secs).await;
        }
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn humidity_does_not_feed_the_range_rule() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        for secs in [0, 20 * MINUTE] {
            alerts.observe(&[
                measured(MAC_BEDROOM, secs, Metric::Humidity, 90.0),
                reading(MAC_OFFICE, secs),
            ]);
        }
        check(&alerts, &notifier, &store, 20 * MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_failed_temperature_send_is_retried() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::failing(1);
        too_hot(&alerts, &notifier, &store).await;
        assert!(notifier.sent.borrow().is_empty());
        assert!(store.saved.borrow().is_empty());
        check(&alerts, &notifier, &store, 16 * MINUTE).await;
        assert_eq!(notifier.titles(), ["bedroom too hot"]);
    }
}
