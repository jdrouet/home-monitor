use std::time::Duration;

use chrono::{NaiveDate, NaiveTime, TimeDelta};
use domain::Timestamp;
use paris_clock::{paris_at, paris_date};
use use_cases::HealthDay;

/// When the daily pull runs, on the Europe/Paris clock.
const PULL_AT: NaiveTime = NaiveTime::from_hms_opt(10, 0, 0).expect("10:00 is a valid time");

/// The stamp of a completed Europe/Paris day: its last microsecond.
pub fn completed_day_stamp(date: NaiveDate) -> Timestamp {
    let next_midnight = paris_at(date + TimeDelta::days(1), NaiveTime::MIN);
    Timestamp::from_micros(next_midnight.as_micros() - 1)
}

/// The days to pull at `now`: today always, stamped `now`; yesterday only from
/// the daily pull time on, stamped at its last microsecond.
pub fn pull_days(now: Timestamp) -> Vec<HealthDay> {
    let today = paris_date(now);
    let mut days = Vec::with_capacity(2);
    if now >= paris_at(today, PULL_AT) {
        let yesterday = today.pred_opt().expect("date is within chrono's range");
        days.push(HealthDay {
            date: yesterday,
            stamp: completed_day_stamp(yesterday),
        });
    }
    days.push(HealthDay {
        date: today,
        stamp: now,
    });
    days
}

/// The next daily pull strictly after `now`.
pub fn next_pull(now: Timestamp) -> Timestamp {
    let today = paris_date(now);
    let at_today = paris_at(today, PULL_AT);
    if now < at_today {
        return at_today;
    }
    let tomorrow = today + TimeDelta::days(1);
    paris_at(tomorrow, PULL_AT)
}

/// How long to sleep from `now` until `next`; zero when `next` has passed.
pub fn wait_until(next: Timestamp, now: Timestamp) -> Duration {
    next.duration_since(now)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{DateTime, NaiveDate};
    use domain::Timestamp;

    use super::{completed_day_stamp, next_pull, pull_days, wait_until};

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    #[test]
    fn a_completed_day_is_stamped_at_its_last_microsecond() {
        assert_eq!(
            completed_day_stamp(date(2026, 1, 14)).as_micros(),
            at("2026-01-14T23:00:00Z").as_micros() - 1
        );
        assert_eq!(
            completed_day_stamp(date(2026, 7, 14)).as_micros(),
            at("2026-07-14T22:00:00Z").as_micros() - 1
        );
    }

    #[test]
    fn dst_days_are_stamped_at_their_own_last_microsecond() {
        assert_eq!(
            completed_day_stamp(date(2026, 3, 29)).as_micros(),
            at("2026-03-29T22:00:00Z").as_micros() - 1
        );
        assert_eq!(
            completed_day_stamp(date(2026, 10, 25)).as_micros(),
            at("2026-10-25T23:00:00Z").as_micros() - 1
        );
    }

    #[test]
    fn before_ten_only_today_is_pulled() {
        let now = at("2026-01-15T08:59:00Z");
        let days = pull_days(now);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, date(2026, 1, 15));
        assert_eq!(days[0].stamp, now);
    }

    #[test]
    fn at_ten_yesterday_joins_stamped_at_its_last_microsecond() {
        let now = at("2026-01-15T09:00:00Z");
        let days = pull_days(now);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, date(2026, 1, 14));
        assert_eq!(
            days[0].stamp.as_micros(),
            at("2026-01-14T23:00:00Z").as_micros() - 1
        );
        assert_eq!(days[1].date, date(2026, 1, 15));
        assert_eq!(days[1].stamp, now);
    }

    #[test]
    fn one_minute_before_and_after_ten_in_summer() {
        assert_eq!(pull_days(at("2026-07-15T07:59:00Z")).len(), 1);
        assert_eq!(pull_days(at("2026-07-15T08:01:00Z")).len(), 2);
    }

    #[test]
    fn just_after_paris_midnight_is_the_new_day() {
        let days = pull_days(at("2026-07-14T22:00:01Z"));
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, date(2026, 7, 15));
    }

    #[test]
    fn dst_days_use_their_own_offset_for_ten() {
        assert_eq!(pull_days(at("2026-03-29T07:59:00Z")).len(), 1);
        assert_eq!(pull_days(at("2026-03-29T08:00:00Z")).len(), 2);
        assert_eq!(pull_days(at("2026-10-25T08:59:00Z")).len(), 1);
        assert_eq!(pull_days(at("2026-10-25T09:00:00Z")).len(), 2);
    }

    #[test]
    fn next_pull_is_today_before_ten_and_tomorrow_from_ten() {
        assert_eq!(
            next_pull(at("2026-01-15T08:59:59Z")),
            at("2026-01-15T09:00:00Z")
        );
        assert_eq!(
            next_pull(at("2026-01-15T09:00:00Z")),
            at("2026-01-16T09:00:00Z")
        );
        assert_eq!(
            next_pull(at("2026-01-15T09:00:01Z")),
            at("2026-01-16T09:00:00Z")
        );
    }

    #[test]
    fn next_pull_crosses_dst_changes() {
        assert_eq!(
            next_pull(at("2026-03-28T12:00:00Z")),
            at("2026-03-29T08:00:00Z")
        );
        assert_eq!(
            next_pull(at("2026-10-24T12:00:00Z")),
            at("2026-10-25T09:00:00Z")
        );
    }

    #[test]
    fn a_pull_that_overruns_the_next_deadline_does_not_wait() {
        let started = at("2026-01-15T08:59:55Z");
        let next = next_pull(started);
        let finished = at("2026-01-15T09:00:05Z");
        assert_eq!(next, at("2026-01-15T09:00:00Z"));
        assert_eq!(wait_until(next, finished), Duration::ZERO);
    }

    #[test]
    fn the_wait_is_the_time_left_before_the_deadline() {
        let next = at("2026-01-15T09:00:00Z");
        assert_eq!(
            wait_until(next, at("2026-01-15T08:59:00Z")),
            Duration::from_secs(60)
        );
    }
}
