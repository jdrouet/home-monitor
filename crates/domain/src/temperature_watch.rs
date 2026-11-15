use std::time::Duration;

use crate::{
    Deviation, REMIND_EVERY, TemperatureAlert, TemperatureEvent, TemperatureRange, Timestamp,
};

/// A room must stay on one side of its range this long before that counts.
pub const TEMPERATURE_HOLD: Duration = Duration::from_secs(15 * 60);

/// Readings on one side of the range, `None` meaning inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Streak {
    deviation: Option<Deviation>,
    since: Timestamp,
    last_at: Timestamp,
    celsius: f64,
}

/// Tracks one room's temperature and which alert was delivered for it.
#[derive(Debug, Clone, PartialEq)]
pub struct TemperatureWatch {
    range: TemperatureRange,
    streak: Option<Streak>,
    active: Option<TemperatureAlert>,
}

impl TemperatureWatch {
    pub const fn new(range: TemperatureRange, restored: Option<TemperatureAlert>) -> Self {
        Self {
            range,
            streak: None,
            active: restored,
        }
    }

    pub const fn range(&self) -> TemperatureRange {
        self.range
    }

    pub const fn active(&self) -> Option<TemperatureAlert> {
        self.active
    }

    /// Records a reading; one not newer than the latest is ignored.
    pub fn saw(&mut self, at: Timestamp, celsius: f64) {
        let deviation = self.range.deviation(celsius);
        match &mut self.streak {
            Some(streak) if at <= streak.last_at => {}
            Some(streak) if streak.deviation == deviation => {
                streak.last_at = at;
                streak.celsius = celsius;
            }
            _ => {
                self.streak = Some(Streak {
                    deviation,
                    since: at,
                    last_at: at,
                    celsius,
                });
            }
        }
    }

    /// The notification owed at `now`, if any. Pure: state changes only
    /// through `delivered`.
    pub fn due(&self, now: Timestamp) -> Option<TemperatureEvent> {
        self.transition().or_else(|| self.reminder(now))
    }

    /// Records that `event` was accepted by the notifier at `now`.
    pub fn delivered(&mut self, event: TemperatureEvent, now: Timestamp) {
        self.active = match event {
            TemperatureEvent::Onset { deviation, celsius } => Some(TemperatureAlert {
                deviation,
                celsius,
                since: now,
                notified_at: now,
            }),
            TemperatureEvent::Reminder { deviation, celsius } => Some(TemperatureAlert {
                deviation,
                celsius,
                since: self.active.map_or(now, |active| active.since),
                notified_at: now,
            }),
            TemperatureEvent::Resolved { .. } => None,
        };
    }

    // A hold is proven by a reading, never by elapsed time alone.
    fn held(&self) -> Option<&Streak> {
        self.streak
            .as_ref()
            .filter(|streak| streak.last_at.duration_since(streak.since) >= TEMPERATURE_HOLD)
    }

    fn transition(&self) -> Option<TemperatureEvent> {
        let streak = self.held()?;
        let celsius = streak.celsius;
        match (streak.deviation, self.active) {
            (Some(deviation), None) => Some(TemperatureEvent::Onset { deviation, celsius }),
            (Some(deviation), Some(active)) if deviation != active.deviation => {
                Some(TemperatureEvent::Onset { deviation, celsius })
            }
            (None, Some(_)) => Some(TemperatureEvent::Resolved { celsius }),
            _ => None,
        }
    }

    fn reminder(&self, now: Timestamp) -> Option<TemperatureEvent> {
        let active = self.active?;
        if now.duration_since(active.notified_at) < REMIND_EVERY {
            return None;
        }
        let celsius = self.streak.map_or(active.celsius, |streak| streak.celsius);
        Some(TemperatureEvent::Reminder {
            deviation: active.deviation,
            celsius,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Deviation, TemperatureAlert, TemperatureEvent, TemperatureRange, Timestamp};

    use super::TemperatureWatch;

    const START: i64 = 1_000_000_000_000;
    const MINUTE: i64 = 60;
    const HOUR: i64 = 3600;
    const HOT: f64 = 21.3;
    const COLD: f64 = 16.5;
    const INSIDE: f64 = 19.0;

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_micros(START + secs * 1_000_000)
    }

    fn range() -> TemperatureRange {
        TemperatureRange::new(18.0, 20.0).expect("valid range")
    }

    fn watch() -> TemperatureWatch {
        TemperatureWatch::new(range(), None)
    }

    fn restored(deviation: Deviation, celsius: f64, notified_at: i64) -> TemperatureWatch {
        TemperatureWatch::new(
            range(),
            Some(TemperatureAlert {
                deviation,
                celsius,
                since: at(notified_at),
                notified_at: at(notified_at),
            }),
        )
    }

    fn onset(deviation: Deviation, celsius: f64) -> Option<TemperatureEvent> {
        Some(TemperatureEvent::Onset { deviation, celsius })
    }

    fn reminder(deviation: Deviation, celsius: f64) -> Option<TemperatureEvent> {
        Some(TemperatureEvent::Reminder { deviation, celsius })
    }

    fn resolved(celsius: f64) -> Option<TemperatureEvent> {
        Some(TemperatureEvent::Resolved { celsius })
    }

    /// Feeds a reading and delivers whatever is due at its time.
    fn feed(watch: &mut TemperatureWatch, secs: i64, celsius: f64) -> Option<TemperatureEvent> {
        watch.saw(at(secs), celsius);
        let event = watch.due(at(secs));
        if let Some(event) = event {
            watch.delivered(event, at(secs));
        }
        event
    }

    #[test]
    fn onset_needs_fifteen_minutes_outside() {
        let mut watch = watch();
        watch.saw(at(0), HOT);
        watch.saw(at(15 * MINUTE - 1), HOT);
        assert_eq!(watch.due(at(15 * MINUTE - 1)), None);
        watch.saw(at(15 * MINUTE), 21.9);
        assert_eq!(watch.due(at(15 * MINUTE)), onset(Deviation::TooHot, 21.9));
    }

    #[test]
    fn too_cold_is_reported_too() {
        let mut watch = watch();
        watch.saw(at(0), COLD);
        watch.saw(at(15 * MINUTE), COLD);
        assert_eq!(watch.due(at(15 * MINUTE)), onset(Deviation::TooCold, COLD));
    }

    #[test]
    fn elapsed_time_alone_proves_nothing() {
        let mut watch = watch();
        watch.saw(at(0), HOT);
        assert_eq!(watch.due(at(HOUR)), None);
    }

    #[test]
    fn a_reading_inside_restarts_the_hold() {
        let mut watch = watch();
        watch.saw(at(0), HOT);
        watch.saw(at(10 * MINUTE), HOT);
        watch.saw(at(11 * MINUTE), INSIDE);
        watch.saw(at(12 * MINUTE), HOT);
        watch.saw(at(26 * MINUTE), HOT);
        assert_eq!(watch.due(at(26 * MINUTE)), None);
        watch.saw(at(27 * MINUTE), HOT);
        assert!(watch.due(at(27 * MINUTE)).is_some());
    }

    #[test]
    fn hovering_at_the_limit_sends_nothing() {
        let mut idle = watch();
        let mut alerting = restored(Deviation::TooHot, HOT, 0);
        for step in 0..=24 {
            let celsius = if step % 2 == 0 { HOT } else { INSIDE };
            idle.saw(at(step * 5 * MINUTE), celsius);
            alerting.saw(at(step * 5 * MINUTE), celsius);
            assert_eq!(idle.due(at(step * 5 * MINUTE)), None);
            assert_eq!(alerting.due(at(step * 5 * MINUTE)), None);
        }
    }

    #[test]
    fn nothing_follows_an_onset_until_the_reminder() {
        let mut watch = watch();
        watch.saw(at(0), HOT);
        assert_eq!(
            feed(&mut watch, 15 * MINUTE, HOT),
            onset(Deviation::TooHot, HOT)
        );
        let sent = 15 * MINUTE;
        assert_eq!(feed(&mut watch, sent + MINUTE, 21.5), None);
        assert_eq!(watch.due(at(sent + 12 * HOUR - 1)), None);
        assert_eq!(
            watch.due(at(sent + 12 * HOUR)),
            reminder(Deviation::TooHot, 21.5)
        );
    }

    #[test]
    fn a_delivered_reminder_restarts_the_timer() {
        let mut watch = restored(Deviation::TooHot, HOT, 0);
        let event = watch.due(at(12 * HOUR)).expect("reminder is due");
        watch.delivered(event, at(12 * HOUR));
        assert_eq!(watch.due(at(24 * HOUR - 1)), None);
        assert!(watch.due(at(24 * HOUR)).is_some());
    }

    #[test]
    fn resolved_needs_fifteen_minutes_inside() {
        let mut watch = restored(Deviation::TooHot, HOT, 0);
        watch.saw(at(0), INSIDE);
        watch.saw(at(10 * MINUTE), INSIDE);
        assert_eq!(watch.due(at(10 * MINUTE)), None);
        watch.saw(at(11 * MINUTE), HOT);
        watch.saw(at(12 * MINUTE), 19.5);
        watch.saw(at(27 * MINUTE - 1), 19.5);
        assert_eq!(watch.due(at(27 * MINUTE - 1)), None);
        watch.saw(at(27 * MINUTE), 19.5);
        assert_eq!(watch.due(at(27 * MINUTE)), resolved(19.5));
    }

    #[test]
    fn a_resolved_alert_stays_quiet() {
        let mut watch = restored(Deviation::TooHot, HOT, 0);
        watch.saw(at(0), INSIDE);
        assert_eq!(feed(&mut watch, 15 * MINUTE, INSIDE), resolved(INSIDE));
        assert_eq!(watch.active(), None);
        assert_eq!(watch.due(at(48 * HOUR)), None);
    }

    #[test]
    fn older_readings_are_ignored() {
        let mut watch = watch();
        watch.saw(at(100), HOT);
        watch.saw(at(50), INSIDE);
        watch.saw(at(100), INSIDE);
        watch.saw(at(100 + 15 * MINUTE), HOT);
        assert_eq!(
            watch.due(at(100 + 15 * MINUTE)),
            onset(Deviation::TooHot, HOT)
        );
    }

    #[test]
    fn a_restored_alert_with_no_readings_reminds_with_its_temperature() {
        let watch = restored(Deviation::TooHot, HOT, 5 * HOUR);
        assert_eq!(watch.due(at(17 * HOUR - 1)), None);
        assert_eq!(watch.due(at(17 * HOUR)), reminder(Deviation::TooHot, HOT));
    }

    #[test]
    fn a_restored_alert_reminds_with_the_new_reading_and_never_repeats_the_onset() {
        let mut watch = restored(Deviation::TooHot, HOT, 0);
        watch.saw(at(HOUR), 22.4);
        assert_eq!(watch.due(at(HOUR)), None);
        assert_eq!(watch.due(at(12 * HOUR)), reminder(Deviation::TooHot, 22.4));
    }

    #[test]
    fn crossing_to_the_other_extreme_switches_direction() {
        for (from, to, celsius) in [
            (Deviation::TooHot, Deviation::TooCold, COLD),
            (Deviation::TooCold, Deviation::TooHot, HOT),
        ] {
            let mut watch = restored(from, 0.0, 0);
            watch.saw(at(HOUR), celsius);
            watch.saw(at(HOUR + 15 * MINUTE - 1), celsius);
            assert_eq!(watch.due(at(HOUR + 15 * MINUTE - 1)), None);
            let switched = feed(&mut watch, HOUR + 15 * MINUTE, celsius);
            assert_eq!(switched, onset(to, celsius));
            assert_eq!(watch.active().map(|alert| alert.deviation), Some(to));
            assert_eq!(feed(&mut watch, HOUR + 20 * MINUTE, celsius), None);
            let later = HOUR + 15 * MINUTE + 12 * HOUR;
            assert_eq!(watch.due(at(later)), reminder(to, celsius));
        }
    }

    #[test]
    fn reminders_keep_the_onset() {
        let mut watch = watch();
        watch.saw(at(0), HOT);
        feed(&mut watch, 15 * MINUTE, HOT);
        let reminder_at = 15 * MINUTE + 12 * HOUR;
        feed(&mut watch, reminder_at, HOT);
        let alert = watch.active().expect("still active");
        assert_eq!(alert.since, at(15 * MINUTE));
        assert_eq!(alert.notified_at, at(reminder_at));
    }

    #[test]
    fn a_direction_switch_starts_a_new_episode() {
        let mut watch = restored(Deviation::TooHot, HOT, 0);
        watch.saw(at(HOUR), COLD);
        feed(&mut watch, HOUR + 15 * MINUTE, COLD);
        let alert = watch.active().expect("switched");
        assert_eq!(alert.since, at(HOUR + 15 * MINUTE));
    }

    #[test]
    fn delivering_stores_the_event_temperature() {
        let mut watch = watch();
        watch.delivered(
            TemperatureEvent::Onset {
                deviation: Deviation::TooHot,
                celsius: HOT,
            },
            at(HOUR),
        );
        assert_eq!(
            watch.active(),
            Some(TemperatureAlert {
                deviation: Deviation::TooHot,
                celsius: HOT,
                since: at(HOUR),
                notified_at: at(HOUR),
            })
        );
        assert_eq!(watch.range(), range());
    }

    #[test]
    fn due_does_not_change_state() {
        let mut watch = restored(Deviation::TooHot, HOT, 0);
        watch.saw(at(0), HOT);
        let before = watch.clone();
        let _ = watch.due(at(24 * HOUR));
        assert_eq!(watch, before);
    }
}
