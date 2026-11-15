use chrono::{NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Europe::Paris;
use domain::Timestamp;

/// The instant when the Europe/Paris clock shows `time` on `date`; the first
/// one when the time occurs twice.
///
/// # Panics
///
/// Panics when `time` does not exist on `date` because a DST change skips it.
pub fn paris_at(date: NaiveDate, time: NaiveTime) -> Timestamp {
    let instant = Paris
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .expect("local time is not skipped by a DST change");
    Timestamp::from_micros(instant.timestamp_micros())
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate, NaiveTime};
    use domain::Timestamp;

    use super::paris_at;

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    fn time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).expect("valid time")
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    #[test]
    fn ten_oclock_on_the_spring_forward_day_is_summer_time() {
        assert_eq!(
            paris_at(date(2026, 3, 29), time(10, 0)),
            at("2026-03-29T08:00:00Z")
        );
    }

    #[test]
    fn ten_oclock_on_the_fall_back_day_is_winter_time() {
        assert_eq!(
            paris_at(date(2026, 10, 25), time(10, 0)),
            at("2026-10-25T09:00:00Z")
        );
    }

    #[test]
    fn an_ambiguous_time_resolves_to_the_first_occurrence() {
        assert_eq!(
            paris_at(date(2026, 10, 25), time(2, 30)),
            at("2026-10-25T00:30:00Z")
        );
    }
}
