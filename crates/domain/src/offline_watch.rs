use std::time::Duration;

use crate::{OfflineEvent, Timestamp};

/// A sensor counts as not reporting once its newest measurement is this old.
pub const OFFLINE_AFTER: Duration = Duration::from_secs(60 * 60);

/// An offline sensor is reminded about this long after the last notification.
pub const REMIND_EVERY: Duration = Duration::from_secs(12 * 60 * 60);

/// Tracks one sensor's silence and which offline notification was delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineWatch {
    last_seen: Timestamp,
    notified_at: Option<Timestamp>,
}

impl OfflineWatch {
    pub const fn new(baseline: Timestamp) -> Self {
        Self {
            last_seen: baseline,
            notified_at: None,
        }
    }

    /// Records a measurement time; it never moves `last_seen` backwards.
    pub fn saw(&mut self, at: Timestamp) {
        self.last_seen = self.last_seen.max(at);
    }

    /// The notification owed at `now`, if any. Pure: state changes only
    /// through `delivered`.
    pub fn due(&self, now: Timestamp) -> Option<OfflineEvent> {
        let silent_for = now.duration_since(self.last_seen);
        match (silent_for >= OFFLINE_AFTER, self.notified_at) {
            (true, None) => Some(OfflineEvent::Offline { silent_for }),
            (true, Some(at)) if now.duration_since(at) >= REMIND_EVERY => {
                Some(OfflineEvent::StillOffline { silent_for })
            }
            (false, Some(_)) => Some(OfflineEvent::Back),
            _ => None,
        }
    }

    /// Records that `event` was accepted by the notifier at `now`.
    pub fn delivered(&mut self, event: OfflineEvent, now: Timestamp) {
        self.notified_at = match event {
            OfflineEvent::Offline { .. } | OfflineEvent::StillOffline { .. } => Some(now),
            OfflineEvent::Back => None,
        };
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{OfflineEvent, Timestamp};

    use super::OfflineWatch;

    const BASELINE: i64 = 1_000_000_000_000;

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_micros(BASELINE + secs * 1_000_000)
    }

    fn hours(h: u64) -> Duration {
        Duration::from_secs(h * 3600)
    }

    fn notified_offline() -> OfflineWatch {
        let mut watch = OfflineWatch::new(at(0));
        let event = watch.due(at(3600)).expect("offline is due");
        watch.delivered(event, at(3600));
        watch
    }

    #[test]
    fn silent_just_under_an_hour_is_fine() {
        assert_eq!(OfflineWatch::new(at(0)).due(at(3599)), None);
    }

    #[test]
    fn silent_for_an_hour_is_offline() {
        assert_eq!(
            OfflineWatch::new(at(0)).due(at(3600)),
            Some(OfflineEvent::Offline {
                silent_for: hours(1)
            })
        );
    }

    #[test]
    fn due_does_not_change_state() {
        let watch = OfflineWatch::new(at(0));
        let _ = watch.due(at(3600));
        assert_eq!(watch, OfflineWatch::new(at(0)));
    }

    #[test]
    fn reminder_comes_twelve_hours_after_delivery() {
        let watch = notified_offline();
        assert_eq!(watch.due(at(3600 + 12 * 3600 - 1)), None);
        assert_eq!(
            watch.due(at(3600 + 12 * 3600)),
            Some(OfflineEvent::StillOffline {
                silent_for: hours(13)
            })
        );
    }

    #[test]
    fn delivered_reminder_restarts_the_timer() {
        let mut watch = notified_offline();
        let reminder_at = at(3600 + 12 * 3600);
        let event = watch.due(reminder_at).expect("reminder is due");
        watch.delivered(event, reminder_at);
        assert_eq!(watch.due(at(3600 + 24 * 3600 - 1)), None);
        assert!(watch.due(at(3600 + 24 * 3600)).is_some());
    }

    #[test]
    fn a_new_measurement_after_an_alert_is_back() {
        let mut watch = notified_offline();
        watch.saw(at(4000));
        assert_eq!(watch.due(at(4001)), Some(OfflineEvent::Back));
    }

    #[test]
    fn delivered_back_clears_the_alert() {
        let mut watch = notified_offline();
        watch.saw(at(4000));
        watch.delivered(OfflineEvent::Back, at(4001));
        assert_eq!(watch.due(at(4002)), None);
    }

    #[test]
    fn older_measurements_do_not_move_last_seen_back() {
        let mut watch = OfflineWatch::new(at(1000));
        watch.saw(at(10));
        assert_eq!(watch.due(at(1000 + 3599)), None);
        assert!(watch.due(at(1000 + 3600)).is_some());
    }

    #[test]
    fn a_sensor_never_alerted_is_never_back() {
        let mut watch = OfflineWatch::new(at(0));
        watch.saw(at(100));
        assert_eq!(watch.due(at(200)), None);
    }
}
