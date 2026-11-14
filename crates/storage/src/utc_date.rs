use chrono::{DateTime, Datelike, NaiveDate};
use domain::Timestamp;

/// The UTC date of `timestamp`, or `None` outside the years 1 to 9999, which
/// the log and day files cannot name.
pub(crate) fn utc_date(timestamp: Timestamp) -> Option<NaiveDate> {
    let date = DateTime::from_timestamp_micros(timestamp.as_micros())?.date_naive();
    (1..=9999).contains(&date.year()).then_some(date)
}

/// Like `utc_date`, but saturating: instants before year 1 map to the
/// earliest date and later ones to the latest.
pub(crate) fn clamped_utc_date(timestamp: Timestamp) -> NaiveDate {
    utc_date(timestamp).unwrap_or(if timestamp.as_micros() < 0 {
        NaiveDate::MIN
    } else {
        NaiveDate::MAX
    })
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use domain::Timestamp;

    use super::{clamped_utc_date, utc_date};

    #[test]
    fn out_of_range_instants_saturate() {
        assert_eq!(
            clamped_utc_date(Timestamp::from_micros(i64::MIN)),
            NaiveDate::MIN
        );
        assert_eq!(
            clamped_utc_date(Timestamp::from_micros(i64::MAX)),
            NaiveDate::MAX
        );
        assert_eq!(
            Some(clamped_utc_date(Timestamp::from_micros(0))),
            NaiveDate::from_ymd_opt(1970, 1, 1)
        );
    }

    const DAY_MICROS: i64 = 86_400_000_000;

    #[test]
    fn epoch_is_1970_01_01() {
        assert_eq!(
            utc_date(Timestamp::from_micros(0)),
            NaiveDate::from_ymd_opt(1970, 1, 1)
        );
    }

    #[test]
    fn the_last_microsecond_belongs_to_its_day() {
        assert_eq!(
            utc_date(Timestamp::from_micros(DAY_MICROS - 1)),
            NaiveDate::from_ymd_opt(1970, 1, 1)
        );
    }

    #[test]
    fn year_10000_and_beyond_have_no_date() {
        let year_10000 = NaiveDate::from_ymd_opt(10_000, 1, 1)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .expect("valid date")
            .and_utc()
            .timestamp_micros();
        assert_eq!(utc_date(Timestamp::from_micros(year_10000)), None);
        assert_eq!(utc_date(Timestamp::from_micros(i64::MAX)), None);
    }
}
