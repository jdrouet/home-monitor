use chrono::{DateTime, NaiveDateTime};
use chrono_tz::Europe::Paris;
use domain::Timestamp;

/// The Europe/Paris wall clock at `at`; `None` for an instant outside the
/// calendar range.
pub fn paris_datetime(at: Timestamp) -> Option<NaiveDateTime> {
    DateTime::from_timestamp_micros(at.as_micros())
        .map(|utc| utc.with_timezone(&Paris).naive_local())
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use domain::Timestamp;

    use super::paris_datetime;

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    #[test]
    fn winter_is_one_hour_ahead_and_summer_two() {
        let winter = paris_datetime(at("2026-01-15T12:30:00Z")).expect("in range");
        assert_eq!(winter.to_string(), "2026-01-15 13:30:00");
        let summer = paris_datetime(at("2026-07-15T23:30:00Z")).expect("in range");
        assert_eq!(summer.to_string(), "2026-07-16 01:30:00");
    }

    #[test]
    fn an_instant_out_of_range_is_none() {
        assert_eq!(paris_datetime(Timestamp::from_micros(i64::MAX)), None);
    }
}
