use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::Duration;

use domain::{
    AlertKind, AlertState, AttemptOutcome, GARMIN, ImportFailure, ImportJob, ImportJobStatus,
    ImportStatus, LoginEvent, LoginWatch, Measurement, Metric, OfflineEvent, OfflineWatch,
    ResolvedAlert, RoomName, SourceId, TemperatureEvent, TemperatureWatch, Thermometer, Timestamp,
};
use tracing::{error, info, warn};

use crate::import_notification::import_notification;
use crate::login_notification::login_notification;
use crate::offline_notification::offline_notification;
use crate::temperature_notification::temperature_notification;
use crate::{AlertStateStore, Notification, Notifier};

#[derive(Clone, Copy)]
enum Rule {
    Offline,
    Temperature,
}

const RULES: [Rule; 2] = [Rule::Offline, Rule::Temperature];

/// Resolved alerts are kept this long after they ended.
const HISTORY_KEPT: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone, Copy)]
enum Event {
    Offline(OfflineEvent),
    Temperature(TemperatureEvent),
}

impl Event {
    const fn rule(self) -> Rule {
        match self {
            Self::Offline(_) => Rule::Offline,
            Self::Temperature(_) => Rule::Temperature,
        }
    }
}

/// An alert from its first notification to its resolution.
#[derive(Clone, Copy, PartialEq)]
struct Episode {
    kind: AlertKind,
    since: Timestamp,
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

    fn episode(&self, rule: Rule) -> Option<Episode> {
        match rule {
            Rule::Offline => self.offline.active().map(|alert| Episode {
                kind: AlertKind::Offline,
                since: alert.since,
            }),
            Rule::Temperature => {
                let alert = self.temperature.as_ref()?.active()?;
                Some(Episode {
                    kind: alert.deviation.into(),
                    since: alert.since,
                })
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
    // Resolved alerts, oldest first.
    history: RefCell<Vec<ResolvedAlert>>,
    login: RefCell<LoginWatch>,
    // The newest give-up notified per Garmin job, as its `retry_at`.
    imports_notified: RefCell<BTreeMap<ImportJob, Timestamp>>,
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
            history: RefCell::new(state.history),
            login: RefCell::new(state.garmin_login.map(LoginWatch::from).unwrap_or_default()),
            imports_notified: RefCell::new(state.garmin_imports),
            unsaved: Cell::new(false),
        }
    }

    /// Records measurements of configured sensors and the Garmin session
    /// status; others are ignored. Every metric shows a sensor is alive; only
    /// temperatures feed the range rule.
    pub fn observe(&self, measurements: &[Measurement]) {
        let mut sensors = self.sensors.borrow_mut();
        for measurement in measurements {
            if self.observe_login(measurement) {
                continue;
            }
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

    // True for a session status reading; a moved watermark is saved at the
    // next check.
    fn observe_login(&self, measurement: &Measurement) -> bool {
        let is_login =
            measurement.metric == Metric::SessionValid && measurement.source_id.as_str() == GARMIN;
        if is_login
            && self
                .login
                .borrow_mut()
                .saw(measurement.timestamp, measurement.value)
        {
            self.unsaved.set(true);
        }
        is_login
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
        if self.prune_history(now()) {
            self.unsaved.set(true);
        }
        if self.unsaved.get() {
            self.save(store, now());
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
                        self.save(store, now());
                    }
                    Err(error) => {
                        warn!(source_id = source_id.as_str(), %error, "notification failed, retrying at the next check");
                    }
                }
            }
        }
        self.notify_login(notifier, store, &now).await;
    }

    /// Sends one notification for each job of `status` that gave up and was
    /// not notified yet. A give-up counts when its `retry_at` is newer than the
    /// one last delivered for the job and still ahead, checked just before the
    /// send. A daily pull refused for its login is left to the login alert. A
    /// failed send is retried by the next report; a failed save by the next
    /// check. `local_time` shows the next try in the owner's time zone.
    pub async fn notify_imports<N: Notifier, S: AlertStateStore, C: Fn() -> Timestamp>(
        &self,
        status: &ImportStatus,
        notifier: &N,
        store: &S,
        now: C,
        local_time: impl Fn(Timestamp) -> String,
    ) {
        for (job, job_status) in &status.jobs {
            let Some((failure, retry_at, retries)) = give_up(*job, job_status) else {
                continue;
            };
            let notification = import_notification(*job, failure, retries, &local_time(retry_at));
            if !self.import_due(*job, retry_at, now()) {
                continue;
            }
            match notifier.notify(&notification).await {
                Ok(()) => {
                    self.imports_notified.borrow_mut().insert(*job, retry_at);
                    self.save(store, now());
                    info!(?job, ?failure, "garmin import notification delivered");
                }
                Err(error) => {
                    warn!(?job, %error, "garmin import notification failed, retrying at the next poll");
                }
            }
        }
    }

    fn import_due(&self, job: ImportJob, retry_at: Timestamp, now: Timestamp) -> bool {
        retry_at > now
            && self
                .imports_notified
                .borrow()
                .get(&job)
                .is_none_or(|notified| retry_at > *notified)
    }

    async fn notify_login<N: Notifier, S: AlertStateStore, C: Fn() -> Timestamp>(
        &self,
        notifier: &N,
        store: &S,
        now: &C,
    ) {
        let Some(event) = self.login.borrow().due(now()) else {
            return;
        };
        match notifier.notify(&login_notification(event)).await {
            Ok(()) => {
                self.login_delivered(event, now());
                self.save(store, now());
            }
            Err(error) => {
                warn!(%error, "garmin login notification failed, retrying at the next check");
            }
        }
    }

    fn login_delivered(&self, event: LoginEvent, now: Timestamp) {
        let mut login = self.login.borrow_mut();
        let since = login.since();
        login.delivered(event, now);
        if let (LoginEvent::Back, Some(since)) = (event, since) {
            self.history.borrow_mut().push(ResolvedAlert {
                source_id: SourceId::from(GARMIN.to_owned()),
                kind: AlertKind::GarminLogin,
                started: since,
                ended: now,
            });
        }
        info!(?event, "garmin login notification delivered");
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
            let rule = event.rule();
            let before = watched.episode(rule);
            watched.delivered(event, now);
            if let Some(ended) = before.filter(|episode| Some(*episode) != watched.episode(rule)) {
                self.history.borrow_mut().push(ResolvedAlert {
                    source_id: source_id.clone(),
                    kind: ended.kind,
                    started: ended.since,
                    ended: now,
                });
            }
            info!(room = %watched.room, ?event, "notification delivered");
        }
    }

    /// Drops history older than `HISTORY_KEPT`; true when something was dropped.
    fn prune_history(&self, now: Timestamp) -> bool {
        let mut history = self.history.borrow_mut();
        let kept = history.len();
        history.retain(|resolved| now.duration_since(resolved.ended) <= HISTORY_KEPT);
        history.len() != kept
    }

    /// The active alerts and the history kept at `now`.
    pub fn state(&self, now: Timestamp) -> AlertState {
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
            history: self
                .history
                .borrow()
                .iter()
                .filter(|resolved| now.duration_since(resolved.ended) <= HISTORY_KEPT)
                .cloned()
                .collect(),
            garmin_login: self.login.borrow().state(),
            garmin_imports: self.imports_notified.borrow().clone(),
        }
    }

    fn save<S: AlertStateStore>(&self, store: &S, now: Timestamp) {
        match store.save(&self.state(now)) {
            Ok(()) => self.unsaved.set(false),
            Err(error) => {
                warn!(%error, "cannot save the alert state, retrying at the next check");
                self.unsaved.set(true);
            }
        }
    }
}

/// The reason, the pull a job waits for and the retries that failed, when its
/// latest attempt ended in a failure the owner has to hear about.
fn give_up(job: ImportJob, status: &ImportJobStatus) -> Option<(ImportFailure, Timestamp, usize)> {
    let end = status.attempts.last()?.ended?;
    let AttemptOutcome::Failed(failure) = end.outcome else {
        return None;
    };
    if status.next_retry.is_some()
        || (job, failure) == (ImportJob::DailyPull, ImportFailure::LoginRejected)
    {
        return None;
    }
    Some((
        failure,
        status.retry_at?,
        status.attempts.len().saturating_sub(1),
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::future::{self, Future};
    use std::io;

    use domain::{
        AlertKind, AlertState, Deviation, GARMIN, LoginState, MacAddress, Measurement, Metric,
        OfflineAlert, ResolvedAlert, RoomName, SourceId, SourceKind, TemperatureAlert,
        TemperatureRange, Thermometer, Timestamp,
    };
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure, ImportJob,
        ImportStatus,
    };

    use crate::{AlertStateStore, Notification, Notifier};

    use super::Alerts;

    const MAC_BEDROOM: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 1]);
    const MAC_OFFICE: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 2]);
    const MAC_STRANGER: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9]);
    const START: i64 = 1_000_000_000_000;
    const MINUTE: i64 = 60;
    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;

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

    fn resolved(mac: MacAddress, kind: AlertKind, started: i64, ended: i64) -> ResolvedAlert {
        ResolvedAlert {
            source_id: mac.into(),
            kind,
            started: at(started),
            ended: at(ended),
        }
    }

    fn holding_history(history: Vec<ResolvedAlert>) -> FakeStore {
        FakeStore::holding(AlertState {
            history,
            ..AlertState::default()
        })
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

    #[tokio::test]
    async fn an_offline_alert_is_recorded_when_back_is_delivered() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR).await;
        assert!(alerts.state(at(HOUR)).history.is_empty());
        alerts.observe(&[reading(MAC_BEDROOM, HOUR + 10)]);
        check(&alerts, &notifier, &store, HOUR + 20).await;
        let expected = [resolved(MAC_BEDROOM, AlertKind::Offline, HOUR, HOUR + 20)];
        assert_eq!(alerts.state(at(HOUR + 20)).history, expected);
        assert_eq!(
            store.saved.borrow().last().expect("saved").history,
            expected
        );
    }

    #[tokio::test]
    async fn a_temperature_alert_is_recorded_when_resolved() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        too_hot(&alerts, &notifier, &store).await;
        live(&alerts, 20 * MINUTE, 19.0);
        live(&alerts, 35 * MINUTE, 19.0);
        check(&alerts, &notifier, &store, 35 * MINUTE).await;
        assert_eq!(
            alerts.state(at(35 * MINUTE)).history,
            [resolved(
                MAC_BEDROOM,
                AlertKind::TooHot,
                15 * MINUTE,
                35 * MINUTE
            )]
        );
    }

    #[tokio::test]
    async fn reminders_keep_the_onset_and_add_no_history() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        too_hot(&alerts, &notifier, &store).await;
        let later = 15 * MINUTE + 12 * HOUR;
        live(&alerts, later, 22.0);
        check(&alerts, &notifier, &store, later).await;
        assert_eq!(notifier.titles()[1], "bedroom still too hot");
        let state = alerts.state(at(later));
        assert_eq!(
            state.temperature[&MAC_BEDROOM.into()].since,
            at(15 * MINUTE)
        );
        assert!(state.history.is_empty());
    }

    #[tokio::test]
    async fn offline_reminders_keep_the_onset_and_add_no_history() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR).await;
        check(&alerts, &notifier, &store, 13 * HOUR).await;
        assert_eq!(notifier.titles()[1], "bedroom thermometer still offline");
        let state = alerts.state(at(13 * HOUR));
        assert_eq!(state.offline[&MAC_BEDROOM.into()].since, at(HOUR));
        assert!(state.history.is_empty());
    }

    #[tokio::test]
    async fn a_direction_switch_ends_one_alert_and_starts_the_next() {
        let store = FakeStore::new();
        let alerts = ranged_alerts(&store, 0);
        let notifier = FakeNotifier::new();
        too_hot(&alerts, &notifier, &store).await;
        live(&alerts, HOUR, 16.0);
        live(&alerts, HOUR + 15 * MINUTE, 16.0);
        check(&alerts, &notifier, &store, HOUR + 15 * MINUTE).await;
        assert_eq!(notifier.titles()[1], "bedroom too cold");
        let state = alerts.state(at(HOUR + 15 * MINUTE));
        assert_eq!(
            state.history,
            [resolved(
                MAC_BEDROOM,
                AlertKind::TooHot,
                15 * MINUTE,
                HOUR + 15 * MINUTE
            )]
        );
        let cold = state.temperature[&MAC_BEDROOM.into()];
        assert_eq!(cold.deviation, Deviation::TooCold);
        assert_eq!(cold.since, at(HOUR + 15 * MINUTE));
    }

    #[tokio::test]
    async fn a_failed_back_send_adds_no_history() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[reading(MAC_OFFICE, 100 * HOUR)]);
        check(&alerts, &notifier, &store, HOUR).await;
        alerts.observe(&[reading(MAC_BEDROOM, HOUR + 10)]);
        notifier.failures_left.set(1);
        check(&alerts, &notifier, &store, HOUR + 20).await;
        assert!(alerts.state(at(HOUR + 20)).history.is_empty());
        check(&alerts, &notifier, &store, HOUR + 80).await;
        assert_eq!(alerts.state(at(HOUR + 80)).history.len(), 1);
    }

    #[test]
    fn history_older_than_thirty_days_is_left_out_of_the_state() {
        let entry = resolved(MAC_BEDROOM, AlertKind::Offline, 0, 0);
        let store = holding_history(vec![entry.clone()]);
        let alerts = alerts(&store, 0);
        assert_eq!(alerts.state(at(30 * DAY)).history, [entry]);
        assert!(alerts.state(at(30 * DAY + 1)).history.is_empty());
    }

    #[tokio::test]
    async fn expired_history_leaves_the_file_even_in_a_quiet_period() {
        let store = holding_history(vec![resolved(MAC_BEDROOM, AlertKind::Offline, 0, 0)]);
        let alerts = alerts(&store, 30 * DAY);
        let notifier = FakeNotifier::new();
        alerts.observe(&[
            reading(MAC_BEDROOM, 30 * DAY),
            reading(MAC_OFFICE, 30 * DAY),
        ]);
        check(&alerts, &notifier, &store, 30 * DAY).await;
        assert!(store.saved.borrow().is_empty());
        check(&alerts, &notifier, &store, 30 * DAY + 1).await;
        assert!(notifier.sent.borrow().is_empty());
        assert_eq!(store.saved.borrow().len(), 1);
        assert!(store.saved.borrow()[0].history.is_empty());
        check(&alerts, &notifier, &store, 30 * DAY + 2).await;
        assert_eq!(store.saved.borrow().len(), 1);
    }

    #[test]
    fn a_restart_returns_the_persisted_history() {
        let history = vec![resolved(MAC_OFFICE, AlertKind::TooCold, 10, 20)];
        let store = holding_history(history.clone());
        assert_eq!(alerts(&store, 100).state(at(100)).history, history);
    }

    fn session(secs: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: at(secs),
            source_kind: SourceKind::Health,
            source_id: SourceId::from(GARMIN.to_owned()),
            metric: Metric::SessionValid,
            value,
        }
    }

    // Both sensors report, then the check runs, so only the login can notify.
    async fn check_live<H: Fn()>(
        alerts: &Alerts,
        notifier: &FakeNotifier<H>,
        store: &FakeStore,
        secs: i64,
    ) {
        live(alerts, secs, 21.0);
        check(alerts, notifier, store, secs).await;
    }

    fn login_state(
        failed_at: Option<i64>,
        ok_at: Option<i64>,
        notified_at: Option<i64>,
    ) -> LoginState {
        LoginState {
            failed_at: failed_at.map(at),
            ok_at: ok_at.map(at),
            since: notified_at.map(at),
            notified_at: notified_at.map(at),
        }
    }

    fn holding_login(garmin_login: LoginState) -> FakeStore {
        FakeStore::holding(AlertState {
            garmin_login: Some(garmin_login),
            ..AlertState::default()
        })
    }

    #[tokio::test]
    async fn garmin_login_lifecycle() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[session(10 * MINUTE, 0.0)]);
        check_live(&alerts, &notifier, &store, 10 * MINUTE).await;
        assert_eq!(notifier.titles(), ["Garmin login expired"]);
        assert!(notifier.messages()[0].contains("health-collector login"));
        check_live(&alerts, &notifier, &store, 12 * HOUR + 9 * MINUTE + 59).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        check_live(&alerts, &notifier, &store, 12 * HOUR + 10 * MINUTE).await;
        assert_eq!(notifier.titles()[1], "Garmin login still expired");
        alerts.observe(&[session(13 * HOUR, 1.0)]);
        check_live(&alerts, &notifier, &store, 13 * HOUR).await;
        assert_eq!(notifier.titles()[2], "Garmin login works again");
        let state = alerts.state(at(13 * HOUR));
        assert_eq!(
            state.history,
            [ResolvedAlert {
                source_id: SourceId::from(GARMIN.to_owned()),
                kind: AlertKind::GarminLogin,
                started: at(10 * MINUTE),
                ended: at(13 * HOUR),
            }]
        );
        check_live(&alerts, &notifier, &store, 14 * HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 3);
    }

    #[tokio::test]
    async fn no_session_reading_means_no_login_notification() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        check_live(&alerts, &notifier, &store, 30 * MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_failed_login_send_is_retried() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::failing(1);
        alerts.observe(&[session(MINUTE, 0.0)]);
        check_live(&alerts, &notifier, &store, MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
        check_live(&alerts, &notifier, &store, 2 * MINUTE).await;
        check_live(&alerts, &notifier, &store, 3 * MINUTE).await;
        assert_eq!(notifier.titles(), ["Garmin login expired"]);
    }

    #[tokio::test]
    async fn a_delivered_login_alert_is_saved_with_both_watermarks() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[session(5, 1.0), session(MINUTE, 0.0)]);
        check_live(&alerts, &notifier, &store, MINUTE).await;
        let saved = store.saved.borrow();
        assert_eq!(
            saved.last().expect("saved").garmin_login,
            Some(login_state(Some(MINUTE), Some(5), Some(MINUTE)))
        );
    }

    #[tokio::test]
    async fn a_moved_watermark_is_saved_at_the_next_check() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        alerts.observe(&[session(MINUTE, 1.0)]);
        check_live(&alerts, &notifier, &store, MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
        assert_eq!(
            store.saved.borrow()[0].garmin_login,
            Some(login_state(None, Some(MINUTE), None))
        );
    }

    #[tokio::test]
    async fn a_restart_keeps_the_schedule_and_does_not_repeat_the_onset() {
        let store = holding_login(login_state(Some(100), None, Some(100)));
        let alerts = alerts(&store, 200);
        let notifier = FakeNotifier::new();
        check_live(&alerts, &notifier, &store, 300).await;
        assert!(notifier.sent.borrow().is_empty());
        check_live(&alerts, &notifier, &store, 100 + 12 * HOUR).await;
        assert_eq!(notifier.titles(), ["Garmin login still expired"]);
        assert_eq!(
            alerts.state(at(100 + 12 * HOUR)).garmin_login,
            Some(LoginState {
                notified_at: Some(at(100 + 12 * HOUR)),
                ..login_state(Some(100), None, Some(100))
            })
        );
    }

    #[tokio::test]
    async fn a_back_that_failed_to_send_is_still_due_after_a_restart() {
        let store = FakeStore::new();
        let first = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        first.observe(&[session(MINUTE, 0.0)]);
        check_live(&first, &notifier, &store, MINUTE).await;
        first.observe(&[session(2 * MINUTE, 1.0)]);
        notifier.failures_left.set(1);
        check_live(&first, &notifier, &store, 3 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        let saved = store.saved.borrow().last().cloned().expect("saved");
        let restarted_store = FakeStore::holding(saved);
        let restarted = alerts(&restarted_store, 4 * MINUTE);
        let notifier = FakeNotifier::new();
        check_live(&restarted, &notifier, &restarted_store, 4 * MINUTE).await;
        assert_eq!(notifier.titles(), ["Garmin login works again"]);
    }

    #[tokio::test]
    async fn an_older_failure_after_a_delivered_back_raises_nothing() {
        let store = holding_login(login_state(Some(100), Some(200), None));
        let alerts = alerts(&store, 300);
        let notifier = FakeNotifier::new();
        alerts.observe(&[session(0, 0.0)]);
        check_live(&alerts, &notifier, &store, 400).await;
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn other_sources_and_metrics_do_not_feed_the_login() {
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let notifier = FakeNotifier::new();
        let mut other_source = session(MINUTE, 0.0);
        other_source.source_id = SourceId::from("other".to_owned());
        let mut other_metric = session(MINUTE, 0.0);
        other_metric.metric = Metric::Steps;
        alerts.observe(&[other_source, other_metric]);
        check_live(&alerts, &notifier, &store, MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
        assert_eq!(alerts.state(at(MINUTE)).garmin_login, None);
    }

    fn local(at: Timestamp) -> String {
        format!("local {}", (at.as_micros() - START) / 1_000_000)
    }

    fn gave_up(records: &[(ImportJob, ImportFailure, i64)]) -> ImportStatus {
        let mut status = ImportStatus::default();
        for (job, failure, retry_secs) in records {
            let attempt = ImportAttempt {
                started: at(0),
                ended: Some(AttemptEnd {
                    at: at(1),
                    outcome: AttemptOutcome::Failed(*failure),
                    counts: ImportCounts::default(),
                }),
            };
            status.record(*job, attempt, at(*retry_secs), None);
        }
        status
    }

    fn workouts_gave_up(retry_secs: i64) -> ImportStatus {
        gave_up(&[(ImportJob::Workouts, ImportFailure::GarminDown, retry_secs)])
    }

    async fn report(
        alerts: &Alerts,
        status: &ImportStatus,
        notifier: &FakeNotifier<impl Fn()>,
        store: &FakeStore,
        now: i64,
    ) {
        alerts
            .notify_imports(status, notifier, store, || at(now), local)
            .await;
    }

    #[tokio::test]
    async fn a_give_up_notifies_once_and_is_saved() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        let status = workouts_gave_up(10 * HOUR);
        report(&alerts, &status, &notifier, &store, 0).await;
        report(&alerts, &status, &notifier, &store, MINUTE).await;
        assert_eq!(
            notifier.titles(),
            ["Garmin import stopped: Workout archive"]
        );
        assert!(notifier.messages()[0].contains("Garmin had a server error."));
        assert!(notifier.messages()[0].ends_with("daily pull, local 36000."));
        let saved = store.saved.borrow();
        assert_eq!(
            saved.last().expect("saved").garmin_imports[&ImportJob::Workouts],
            at(10 * HOUR)
        );
    }

    #[tokio::test]
    async fn a_newer_retry_time_notifies_again_and_an_older_one_does_not() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        report(&alerts, &workouts_gave_up(10 * HOUR), &notifier, &store, 0).await;
        report(
            &alerts,
            &workouts_gave_up(34 * HOUR),
            &notifier,
            &store,
            11 * HOUR,
        )
        .await;
        report(
            &alerts,
            &workouts_gave_up(20 * HOUR),
            &notifier,
            &store,
            12 * HOUR,
        )
        .await;
        assert_eq!(notifier.sent.borrow().len(), 2);
    }

    #[tokio::test]
    async fn a_retry_time_in_the_past_sends_nothing() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        report(
            &alerts,
            &workouts_gave_up(10 * HOUR),
            &notifier,
            &store,
            10 * HOUR,
        )
        .await;
        report(
            &alerts,
            &workouts_gave_up(10 * HOUR),
            &notifier,
            &store,
            11 * HOUR,
        )
        .await;
        assert!(notifier.sent.borrow().is_empty());
        assert!(store.saved.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_restart_with_the_saved_state_does_not_repeat_a_give_up() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let first = alerts(&store, 0);
        let status = workouts_gave_up(10 * HOUR);
        report(&first, &status, &notifier, &store, 0).await;
        let restarted_store =
            FakeStore::holding(store.saved.borrow().last().expect("saved").clone());
        let restarted = alerts(&restarted_store, HOUR);
        report(&restarted, &status, &notifier, &restarted_store, HOUR).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
    }

    fn record_try(
        status: &mut ImportStatus,
        started: i64,
        outcome: Option<AttemptOutcome>,
        retry: Option<i64>,
    ) {
        let attempt = ImportAttempt {
            started: at(started),
            ended: outcome.map(|outcome| AttemptEnd {
                at: at(started + 1),
                outcome,
                counts: ImportCounts::default(),
            }),
        };
        status.record(ImportJob::Workouts, attempt, at(10 * HOUR), retry.map(at));
    }

    fn down() -> AttemptOutcome {
        AttemptOutcome::Failed(ImportFailure::GarminDown)
    }

    /// Records the three retries of a failing job one step at a time and
    /// reports after each step; the last one gives up.
    async fn retry_until_give_up(
        alerts: &Alerts,
        status: &mut ImportStatus,
        notifier: &FakeNotifier<impl Fn()>,
        store: &FakeStore,
    ) -> usize {
        let starts = [0, 5 * MINUTE, 20 * MINUTE, 80 * MINUTE];
        let mut before_the_end = 0;
        for (k, start) in starts.into_iter().enumerate() {
            record_try(status, start, None, None);
            report(alerts, status, notifier, store, start).await;
            let retry = starts.get(k + 1).copied();
            record_try(status, start, Some(down()), retry);
            if retry.is_some() {
                report(alerts, status, notifier, store, start + MINUTE).await;
                before_the_end = notifier.sent.borrow().len();
            }
        }
        before_the_end
    }

    #[tokio::test]
    async fn a_job_that_retries_then_gives_up_notifies_once_after_the_last_retry() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        let mut status = ImportStatus::default();
        let before_the_end = retry_until_give_up(&alerts, &mut status, &notifier, &store).await;
        assert_eq!(before_the_end, 0);
        assert!(notifier.sent.borrow().is_empty());
        report(&alerts, &status, &notifier, &store, 81 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        for poll in [82, 83] {
            report(&alerts, &status, &notifier, &store, poll * MINUTE).await;
        }
        assert_eq!(notifier.sent.borrow().len(), 1);
        assert!(notifier.messages()[0].contains("It retried 3 times."));
    }

    #[tokio::test]
    async fn a_report_with_both_a_planned_retry_and_a_give_up_time_sends_nothing() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        let mut status = workouts_gave_up(10 * HOUR);
        status
            .jobs
            .get_mut(&ImportJob::Workouts)
            .expect("job")
            .next_retry = Some(at(5 * MINUTE));
        report(&alerts, &status, &notifier, &store, MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
        assert!(store.saved.borrow().is_empty());
        status
            .jobs
            .get_mut(&ImportJob::Workouts)
            .expect("job")
            .next_retry = None;
        report(&alerts, &status, &notifier, &store, 2 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_retry_that_works_sends_nothing() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        let mut status = ImportStatus::default();
        record_try(&mut status, 0, Some(down()), Some(5 * MINUTE));
        report(&alerts, &status, &notifier, &store, MINUTE).await;
        record_try(&mut status, 5 * MINUTE, None, None);
        record_try(&mut status, 5 * MINUTE, Some(AttemptOutcome::Done), None);
        report(&alerts, &status, &notifier, &store, 7 * MINUTE).await;
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_restart_during_the_backoff_then_the_give_up_notifies_once() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let first = alerts(&store, 0);
        let mut status = ImportStatus::default();
        record_try(&mut status, 0, Some(down()), Some(5 * MINUTE));
        report(&first, &status, &notifier, &store, MINUTE).await;
        let restarted = alerts(&store, 2 * MINUTE);
        record_try(&mut status, 5 * MINUTE, None, None);
        record_try(&mut status, 5 * MINUTE, Some(down()), None);
        report(&restarted, &status, &notifier, &store, 7 * MINUTE).await;
        report(&restarted, &status, &notifier, &store, 8 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        assert!(notifier.messages()[0].contains("It retried once."));
    }

    #[tokio::test]
    async fn an_old_collector_alert_then_a_new_collector_give_up_in_the_window_notifies_once() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        let mut old = ImportStatus::default();
        record_try(&mut old, 0, Some(down()), None);
        report(&alerts, &old, &notifier, &store, MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
        let mut new = ImportStatus::default();
        retry_until_give_up(&alerts, &mut new, &notifier, &store).await;
        report(&alerts, &new, &notifier, &store, 81 * MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
    }

    #[tokio::test]
    async fn only_a_refused_login_of_the_daily_pull_is_left_to_the_login_alert() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        let refused = gave_up(&[(ImportJob::DailyPull, ImportFailure::LoginRejected, HOUR)]);
        report(&alerts, &refused, &notifier, &store, 0).await;
        assert!(notifier.sent.borrow().is_empty());
        let others = gave_up(&[
            (ImportJob::DailyPull, ImportFailure::Unreachable, HOUR),
            (ImportJob::MissedDays, ImportFailure::LoginRejected, HOUR),
            (
                ImportJob::HealthHistory,
                ImportFailure::ProgressUnreadable,
                HOUR,
            ),
        ]);
        report(&alerts, &others, &notifier, &store, 0).await;
        assert_eq!(notifier.sent.borrow().len(), 3);
    }

    #[tokio::test]
    async fn running_paused_and_done_jobs_send_nothing() {
        let (notifier, store) = (FakeNotifier::new(), FakeStore::new());
        let alerts = alerts(&store, 0);
        let mut status = ImportStatus::default();
        let running = ImportAttempt {
            started: at(0),
            ended: None,
        };
        status.record(ImportJob::Workouts, running, at(HOUR), None);
        for (job, outcome) in [
            (ImportJob::MissedDays, AttemptOutcome::Paused),
            (ImportJob::DailyArchive, AttemptOutcome::Done),
        ] {
            let ended = ImportAttempt {
                started: at(0),
                ended: Some(AttemptEnd {
                    at: at(1),
                    outcome,
                    counts: ImportCounts::default(),
                }),
            };
            status.record(job, ended, at(HOUR), None);
        }
        report(&alerts, &status, &notifier, &store, 0).await;
        assert!(notifier.sent.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_failed_send_is_retried_by_the_next_report() {
        let (notifier, store) = (FakeNotifier::failing(1), FakeStore::new());
        let alerts = alerts(&store, 0);
        let status = workouts_gave_up(10 * HOUR);
        report(&alerts, &status, &notifier, &store, 0).await;
        assert!(notifier.sent.borrow().is_empty());
        assert!(store.saved.borrow().is_empty());
        report(&alerts, &status, &notifier, &store, MINUTE).await;
        assert_eq!(notifier.sent.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_retry_time_that_passes_while_an_earlier_send_runs_is_not_sent() {
        let clock = Cell::new(0);
        let notifier = FakeNotifier::with_hook(|| clock.set(2 * HOUR));
        let store = FakeStore::new();
        let alerts = alerts(&store, 0);
        let status = gave_up(&[
            (ImportJob::MissedDays, ImportFailure::GarminDown, HOUR),
            (ImportJob::Workouts, ImportFailure::GarminDown, HOUR),
        ]);
        alerts
            .notify_imports(&status, &notifier, &store, || at(clock.get()), local)
            .await;
        assert_eq!(
            notifier.titles(),
            ["Garmin import stopped: Missed days (last 30)"]
        );
    }
}
