use chrono::NaiveDate;
use domain::Timestamp;

/// A calendar day and the instants it spans: `start` inclusive, `end`
/// exclusive. Its length is not always 24 hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarDay {
    pub date: NaiveDate,
    pub start: Timestamp,
    pub end: Timestamp,
}
