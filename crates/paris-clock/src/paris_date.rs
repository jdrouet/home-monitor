use chrono::{DateTime, NaiveDate};
use chrono_tz::Europe::Paris;
use domain::Timestamp;

/// The Europe/Paris calendar date containing `at`.
pub fn paris_date(at: Timestamp) -> NaiveDate {
    DateTime::from_timestamp_micros(at.as_micros())
        .expect("timestamp is within chrono's range")
        .with_timezone(&Paris)
        .date_naive()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate};
    use domain::Timestamp;

    use super::paris_date;

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    #[test]
    fn utc_evening_is_already_the_next_paris_day() {
        assert_eq!(paris_date(at("2026-07-14T21:59:59Z")), date(2026, 7, 14));
        assert_eq!(paris_date(at("2026-07-14T22:00:00Z")), date(2026, 7, 15));
        assert_eq!(paris_date(at("2026-01-14T22:59:59Z")), date(2026, 1, 14));
        assert_eq!(paris_date(at("2026-01-14T23:00:00Z")), date(2026, 1, 15));
    }
}
