use chrono::DateTime;
use chrono_tz::Europe::Paris;
use domain::Timestamp;

/// `at` as `YYYY-MM-DD HH:MM` on the Europe/Paris clock.
pub fn paris_time(at: Timestamp) -> String {
    DateTime::from_timestamp_micros(at.as_micros())
        .expect("timestamp is within chrono's range")
        .with_timezone(&Paris)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use domain::Timestamp;

    use super::paris_time;

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
}
