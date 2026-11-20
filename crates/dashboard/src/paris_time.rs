use domain::Timestamp;
use paris_clock::paris_datetime;

/// `at` as `YYYY-MM-DD HH:MM` on the Europe/Paris clock; `—` for an instant
/// outside the calendar range.
pub fn paris_time(at: Timestamp) -> String {
    format_paris(at, "%Y-%m-%d %H:%M")
}

/// `at` as `HH:MM` on the Europe/Paris clock; `—` for an instant outside the
/// calendar range.
pub fn paris_hour_minute(at: Timestamp) -> String {
    format_paris(at, "%H:%M")
}

/// `at` as `YYYY-MM-DD` on the Europe/Paris clock; `—` for an instant outside
/// the calendar range.
pub fn paris_day(at: Timestamp) -> String {
    format_paris(at, "%Y-%m-%d")
}

fn format_paris(at: Timestamp, format: &str) -> String {
    paris_datetime(at).map_or_else(|| "—".to_owned(), |time| time.format(format).to_string())
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use domain::Timestamp;

    use super::{paris_day, paris_hour_minute, paris_time};

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    #[test]
    fn winter_time_is_one_hour_ahead_of_utc() {
        assert_eq!(paris_time(at("2026-01-15T12:30:00Z")), "2026-01-15 13:30");
    }

    #[test]
    fn summer_time_is_two_hours_ahead_of_utc() {
        assert_eq!(paris_time(at("2026-07-15T23:30:00Z")), "2026-07-16 01:30");
    }

    #[test]
    fn an_instant_out_of_range_does_not_panic() {
        assert_eq!(paris_time(Timestamp::from_micros(i64::MAX)), "—");
    }

    #[test]
    fn the_day_is_the_paris_day() {
        assert_eq!(paris_day(at("2026-07-15T23:30:00Z")), "2026-07-16");
    }

    #[test]
    fn the_hour_and_minute_follow_winter_time() {
        assert_eq!(paris_hour_minute(at("2026-01-15T12:30:00Z")), "13:30");
    }
}
