use crate::{LoginEvent, LoginState, REMIND_EVERY, Timestamp};

/// Tracks the Garmin login from `session_valid` readings. The login is broken
/// while the newest failure is newer than the newest success, so the order in
/// which readings arrive does not matter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginWatch {
    failed_at: Option<Timestamp>,
    ok_at: Option<Timestamp>,
    notified: Option<Notified>,
}

/// The delivered alert: when its episode began and when it was last notified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Notified {
    since: Timestamp,
    at: Timestamp,
}

impl LoginWatch {
    /// Records a reading, 0 for a failed session and 1 for a working one. A
    /// watermark never moves backwards; true when one moved.
    pub fn saw(&mut self, at: Timestamp, value: f64) -> bool {
        let mark = if value < 0.5 {
            &mut self.failed_at
        } else {
            &mut self.ok_at
        };
        let moved = Some(at) > *mark;
        if moved {
            *mark = Some(at);
        }
        moved
    }

    /// The notification owed at `now`, if any. Pure: state changes only
    /// through `delivered`.
    pub fn due(&self, now: Timestamp) -> Option<LoginEvent> {
        match (self.failed_at > self.ok_at, self.notified) {
            (true, None) => Some(LoginEvent::Expired),
            (true, Some(n)) if now.duration_since(n.at) >= REMIND_EVERY => {
                Some(LoginEvent::StillExpired)
            }
            (false, Some(_)) => Some(LoginEvent::Back),
            _ => None,
        }
    }

    /// Records that `event` was accepted by the notifier at `now`.
    pub fn delivered(&mut self, event: LoginEvent, now: Timestamp) {
        self.notified = match event {
            LoginEvent::Expired => Some(Notified {
                since: now,
                at: now,
            }),
            LoginEvent::StillExpired => Some(Notified {
                since: self.notified.map_or(now, |n| n.since),
                at: now,
            }),
            LoginEvent::Back => None,
        };
    }

    /// When the delivered alert began; none when no alert is delivered.
    pub fn since(&self) -> Option<Timestamp> {
        self.notified.map(|notified| notified.since)
    }

    /// What to keep across restarts; none while the watch is untouched.
    pub fn state(&self) -> Option<LoginState> {
        let state = LoginState {
            failed_at: self.failed_at,
            ok_at: self.ok_at,
            since: self.notified.map(|n| n.since),
            notified_at: self.notified.map(|n| n.at),
        };
        (state != LoginState::default()).then_some(state)
    }
}

impl From<LoginState> for LoginWatch {
    fn from(state: LoginState) -> Self {
        Self {
            failed_at: state.failed_at,
            ok_at: state.ok_at,
            notified: state
                .since
                .zip(state.notified_at)
                .map(|(since, at)| Notified { since, at }),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{LoginEvent, LoginState, Timestamp};

    use super::LoginWatch;

    const HOUR: i64 = 3_600_000_000;

    fn at(micros: i64) -> Timestamp {
        Timestamp::from_micros(micros)
    }

    fn expired(failed: i64) -> LoginWatch {
        let mut watch = LoginWatch::default();
        watch.saw(at(failed), 0.0);
        let event = watch.due(at(failed)).expect("due");
        watch.delivered(event, at(failed));
        watch
    }

    #[test]
    fn an_untouched_watch_owes_nothing_and_keeps_nothing() {
        let watch = LoginWatch::default();
        assert_eq!(watch.due(at(0)), None);
        assert_eq!(watch.state(), None);
    }

    #[test]
    fn a_failure_is_due_at_once() {
        let mut watch = LoginWatch::default();
        assert!(watch.saw(at(5), 0.0));
        assert_eq!(watch.due(at(5)), Some(LoginEvent::Expired));
    }

    #[test]
    fn a_success_alone_owes_nothing() {
        let mut watch = LoginWatch::default();
        assert!(watch.saw(at(5), 1.0));
        assert_eq!(watch.due(at(5)), None);
    }

    #[test]
    fn an_older_success_does_not_end_the_alert_and_a_newer_one_does() {
        let mut watch = expired(100);
        watch.saw(at(50), 1.0);
        assert_eq!(watch.due(at(101)), None);
        watch.saw(at(200), 1.0);
        assert_eq!(watch.due(at(201)), Some(LoginEvent::Back));
    }

    #[test]
    fn a_backlog_failure_then_a_newer_success_owes_nothing() {
        let mut watch = LoginWatch::default();
        watch.saw(at(10), 0.0);
        watch.saw(at(20), 1.0);
        assert_eq!(watch.due(at(30)), None);
    }

    #[test]
    fn watermarks_only_move_forward() {
        let mut watch = LoginWatch::default();
        assert!(watch.saw(at(10), 0.0));
        assert!(!watch.saw(at(10), 0.0));
        assert!(!watch.saw(at(5), 0.0));
        assert!(watch.saw(at(11), 0.0));
    }

    #[test]
    fn the_reminder_is_due_after_exactly_twelve_hours() {
        let watch = expired(0);
        assert_eq!(watch.due(at(12 * HOUR - 1)), None);
        assert_eq!(watch.due(at(12 * HOUR)), Some(LoginEvent::StillExpired));
    }

    #[test]
    fn a_reminder_keeps_the_episode_start() {
        let mut watch = expired(0);
        watch.delivered(LoginEvent::StillExpired, at(12 * HOUR));
        assert_eq!(watch.since(), Some(at(0)));
        assert_eq!(watch.due(at(23 * HOUR)), None);
        assert_eq!(watch.due(at(24 * HOUR)), Some(LoginEvent::StillExpired));
    }

    #[test]
    fn a_delivered_back_ends_the_episode() {
        let mut watch = expired(100);
        watch.saw(at(200), 1.0);
        watch.delivered(LoginEvent::Back, at(201));
        assert_eq!(watch.since(), None);
        assert_eq!(watch.due(at(202)), None);
    }

    #[test]
    fn due_does_not_change_the_watch() {
        let watch = expired(0);
        let before = watch;
        let _ = watch.due(at(13 * HOUR));
        assert_eq!(watch, before);
    }

    #[test]
    fn the_state_round_trips_with_both_watermarks() {
        let mut watch = expired(100);
        watch.saw(at(200), 1.0);
        let state = watch.state().expect("kept");
        assert_eq!(
            state,
            LoginState {
                failed_at: Some(at(100)),
                ok_at: Some(at(200)),
                since: Some(at(100)),
                notified_at: Some(at(100)),
            }
        );
        assert_eq!(LoginWatch::from(state), watch);
    }

    #[test]
    fn a_restored_back_is_still_due_and_an_older_failure_raises_nothing_after_it() {
        let mut watch = expired(100);
        watch.saw(at(200), 1.0);
        let mut restored = LoginWatch::from(watch.state().expect("kept"));
        assert_eq!(restored.due(at(201)), Some(LoginEvent::Back));
        restored.delivered(LoginEvent::Back, at(201));
        let mut again = LoginWatch::from(restored.state().expect("kept"));
        assert!(!again.saw(at(0), 0.0));
        assert_eq!(again.due(at(300)), None);
    }
}
