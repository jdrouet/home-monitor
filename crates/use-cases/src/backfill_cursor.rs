use chrono::{Days, NaiveDate};

use crate::HealthDay;

/// How many days before today the gap fill reaches back, at most.
const GAP_DAYS: u64 = 30;

/// The days of a health import still to fetch, `next..=last`.
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

    /// The days the daily pulls missed, after a completed pull on `today`.
    /// `requested` are the days the pull asked for and `fetched` those that
    /// worked. The range ends at the newest final day (yesterday when it was
    /// fetched, else the day before) and never reaches back more than
    /// `GAP_DAYS` days; yesterday stays owed when the pull asked for it and
    /// failed.
    pub fn after_pull(
        previous: Option<Self>,
        today: NaiveDate,
        requested: &[HealthDay],
        fetched: &[NaiveDate],
    ) -> Self {
        let yesterday = today - Days::new(1);
        let final_day = today - Days::new(2);
        let floor = today - Days::new(GAP_DAYS + 1);
        let newest_final = fetched
            .iter()
            .copied()
            .filter(|day| *day < today)
            .max()
            .unwrap_or(final_day);
        let owes_yesterday =
            requested.iter().any(|day| day.date == yesterday) && !fetched.contains(&yesterday);
        let start = previous.map_or(floor, |cursor| cursor.next.max(floor));
        let mut last = previous.map_or(newest_final, |cursor| cursor.last.max(newest_final));
        if owes_yesterday {
            last = last.max(yesterday);
        }
        let next = if owes_yesterday {
            start.min(yesterday)
        } else if start > final_day {
            last + Days::new(1)
        } else {
            start
        };
        Self { next, last }
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

    use crate::HealthDay;

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

    fn today() -> NaiveDate {
        date(2026, 11, 15)
    }

    // The day `back` days before today.
    fn day(back: u64) -> NaiveDate {
        today() - chrono::Days::new(back)
    }

    fn cursor(next_back: u64, last_back: u64) -> BackfillCursor {
        BackfillCursor {
            next: day(next_back),
            last: day(last_back),
        }
    }

    fn requested(dates: &[NaiveDate]) -> Vec<HealthDay> {
        dates
            .iter()
            .map(|date| HealthDay {
                date: *date,
                stamp: domain::Timestamp::from_micros(0),
            })
            .collect()
    }

    // A pull at or after 10:00 asks for yesterday and today.
    fn after_ten(previous: Option<BackfillCursor>) -> BackfillCursor {
        let days = [day(1), day(0)];
        BackfillCursor::after_pull(previous, today(), &requested(&days), &days)
    }

    // A pull before 10:00 asks for today only.
    fn before_ten(previous: Option<BackfillCursor>) -> BackfillCursor {
        let days = [day(0)];
        BackfillCursor::after_pull(previous, today(), &requested(&days), &days)
    }

    #[test]
    fn the_first_record_owes_thirty_days_up_to_the_newest_final_day() {
        assert_eq!(after_ten(None), cursor(31, 1));
        assert_eq!(before_ten(None), cursor(31, 2));
    }

    #[test]
    fn a_caught_up_cursor_moves_on_each_day_and_stays_when_restarted() {
        assert_eq!(after_ten(Some(cursor(1, 2))), cursor(0, 1));
        assert_eq!(after_ten(Some(cursor(0, 1))), cursor(0, 1));
    }

    #[test]
    fn an_outage_owes_the_days_since_the_last_one() {
        assert_eq!(after_ten(Some(cursor(5, 6))), cursor(5, 1));
    }

    #[test]
    fn the_gap_never_reaches_back_more_than_thirty_days() {
        assert_eq!(after_ten(Some(cursor(40, 2))), cursor(31, 1));
        assert_eq!(after_ten(Some(cursor(100, 101))), cursor(31, 1));
    }

    #[test]
    fn pending_days_are_kept() {
        assert_eq!(before_ten(Some(cursor(10, 2))), cursor(10, 2));
        assert_eq!(after_ten(Some(cursor(10, 2))), cursor(10, 1));
    }

    #[test]
    fn a_recovery_before_ten_owes_up_to_two_days_back_then_ten_oclock_catches_up() {
        let owed = before_ten(Some(cursor(5, 6)));
        assert_eq!(owed, cursor(5, 2));
        let repaired = cursor(1, 2);
        assert_eq!(after_ten(Some(repaired)), cursor(0, 1));
    }

    #[test]
    fn a_failed_yesterday_stays_owed() {
        let days = [day(1), day(0)];
        let fetched = [day(0)];
        let owed =
            |previous| BackfillCursor::after_pull(previous, today(), &requested(&days), &fetched);
        assert_eq!(owed(Some(cursor(1, 2))), cursor(1, 1));
        assert_eq!(owed(Some(cursor(0, 1))), cursor(1, 1));
        assert_eq!(owed(None), cursor(31, 1));
    }

    #[test]
    fn a_failed_today_does_not_change_the_range() {
        let days = [day(0)];
        let planned = BackfillCursor::after_pull(None, today(), &requested(&days), &[]);
        assert_eq!(planned, cursor(31, 2));
    }
}
