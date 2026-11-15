use chrono::NaiveTime;
use domain::Timestamp;

use crate::{paris_at, paris_date};

/// Midnight of the Europe/Paris day containing `now`.
pub fn paris_day_start(now: Timestamp) -> Timestamp {
    paris_at(paris_date(now), NaiveTime::MIN)
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use domain::Timestamp;

    use super::paris_day_start;

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    #[test]
    fn winter_day_starts_an_hour_before_utc_midnight() {
        assert_eq!(
            paris_day_start(at("2026-01-15T12:00:00Z")),
            at("2026-01-14T23:00:00Z")
        );
    }

    #[test]
    fn summer_day_starts_two_hours_before_utc_midnight() {
        assert_eq!(
            paris_day_start(at("2026-07-15T12:00:00Z")),
            at("2026-07-14T22:00:00Z")
        );
    }

    #[test]
    fn instant_just_after_midnight_starts_the_new_day() {
        assert_eq!(
            paris_day_start(at("2026-07-14T22:00:01Z")),
            at("2026-07-14T22:00:00Z")
        );
        assert_eq!(
            paris_day_start(at("2026-07-14T21:59:59Z")),
            at("2026-07-13T22:00:00Z")
        );
    }

    #[test]
    fn day_of_a_dst_change_starts_at_the_old_offset() {
        assert_eq!(
            paris_day_start(at("2026-03-29T12:00:00Z")),
            at("2026-03-28T23:00:00Z")
        );
    }
}
