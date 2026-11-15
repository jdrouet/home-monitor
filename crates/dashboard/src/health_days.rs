use chrono::{NaiveTime, TimeDelta};
use domain::Timestamp;
use paris_clock::{paris_at, paris_date};
use use_cases::CalendarDay;

/// The last `count` Europe/Paris days up to and including today, newest first.
pub fn health_days(now: Timestamp, count: usize) -> Vec<CalendarDay> {
    let today = paris_date(now);
    (0..count)
        .map(|age| {
            let date = today - TimeDelta::days(i64::try_from(age).unwrap_or(i64::MAX));
            CalendarDay {
                date,
                start: paris_at(date, NaiveTime::MIN),
                end: paris_at(date + TimeDelta::days(1), NaiveTime::MIN),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate};
    use domain::Timestamp;

    use super::health_days;

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    fn hours(day: &use_cases::CalendarDay) -> i64 {
        (day.end.as_micros() - day.start.as_micros()) / 3_600_000_000
    }

    #[test]
    fn days_are_newest_first_and_contiguous() {
        let days = health_days(at("2026-07-15T12:00:00Z"), 3);
        let dates: Vec<_> = days.iter().map(|day| day.date).collect();
        let expected = |day| NaiveDate::from_ymd_opt(2026, 7, day).expect("valid date");
        assert_eq!(dates, [expected(15), expected(14), expected(13)]);
        assert_eq!(days[0].start, at("2026-07-14T22:00:00Z"));
        assert_eq!(days[0].end, at("2026-07-15T22:00:00Z"));
        assert_eq!(days[1].end, days[0].start);
        assert_eq!(days[2].end, days[1].start);
    }

    #[test]
    fn today_follows_the_paris_date_not_the_utc_date() {
        let days = health_days(at("2026-07-14T22:30:00Z"), 1);
        assert_eq!(
            days[0].date,
            NaiveDate::from_ymd_opt(2026, 7, 15).expect("valid date")
        );
    }

    #[test]
    fn dst_days_last_23_and_25_hours() {
        let spring = health_days(at("2026-03-29T12:00:00Z"), 1);
        assert_eq!(hours(&spring[0]), 23);
        let autumn = health_days(at("2026-10-25T12:00:00Z"), 1);
        assert_eq!(hours(&autumn[0]), 25);
    }

    #[test]
    fn zero_days_is_empty() {
        assert!(health_days(at("2026-07-15T12:00:00Z"), 0).is_empty());
    }
}
