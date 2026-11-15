use chrono::{NaiveTime, TimeDelta};
use domain::Timestamp;
use paris_clock::{paris_at, paris_date};
use use_cases::HealthDay;

/// When the daily pull runs, on the Europe/Paris clock.
const PULL_AT: NaiveTime = NaiveTime::from_hms_opt(10, 0, 0).expect("10:00 is a valid time");

/// The days to pull at `now`: today always, stamped `now`; yesterday only from
/// the daily pull time on, stamped at its last microsecond.
pub fn pull_days(now: Timestamp) -> Vec<HealthDay> {
    let today = paris_date(now);
    let midnight = paris_at(today, NaiveTime::MIN);
    let mut days = Vec::with_capacity(2);
    if now >= paris_at(today, PULL_AT) {
        days.push(HealthDay {
            date: today.pred_opt().expect("date is within chrono's range"),
            stamp: Timestamp::from_micros(midnight.as_micros() - 1),
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

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate};
    use domain::Timestamp;

    use super::{next_pull, pull_days};

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
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
}
