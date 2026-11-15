use chrono::{Days, NaiveDate};

/// The days of the health backfill still to import, `next..=last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillCursor {
    pub next: NaiveDate,
    pub last: NaiveDate,
}

impl BackfillCursor {
    /// The year before `today`, ending two days back: yesterday belongs to the
    /// daily pull.
    pub fn anchored_at(today: NaiveDate) -> Self {
        Self {
            next: today - Days::new(365),
            last: today - Days::new(2),
        }
    }

    pub fn is_done(&self) -> bool {
        self.next > self.last
    }

    #[must_use]
    pub fn advanced(self) -> Self {
        Self {
            next: self.next + Days::new(1),
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::BackfillCursor;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    fn days_left(mut cursor: BackfillCursor) -> usize {
        let mut count = 0;
        while !cursor.is_done() {
            cursor = cursor.advanced();
            count += 1;
        }
        count
    }

    #[test]
    fn the_range_ends_two_days_before_today() {
        let cursor = BackfillCursor::anchored_at(date(2026, 11, 15));
        assert_eq!(cursor.next, date(2025, 11, 15));
        assert_eq!(cursor.last, date(2026, 11, 13));
        assert_eq!(days_left(cursor), 364);
    }

    #[test]
    fn a_leap_year_moves_the_start_by_a_day() {
        let cursor = BackfillCursor::anchored_at(date(2024, 3, 1));
        assert_eq!(cursor.next, date(2023, 3, 2));
        assert_eq!(cursor.last, date(2024, 2, 28));
    }

    #[test]
    fn a_fresh_cursor_is_not_done() {
        assert!(!BackfillCursor::anchored_at(date(2026, 11, 15)).is_done());
    }

    #[test]
    fn advancing_past_last_is_done() {
        let cursor = BackfillCursor {
            next: date(2026, 1, 2),
            last: date(2026, 1, 2),
        };
        assert!(!cursor.is_done());
        assert!(cursor.advanced().is_done());
    }
}
