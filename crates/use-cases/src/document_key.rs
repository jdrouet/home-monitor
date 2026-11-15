use chrono::NaiveDate;

use crate::ActivityId;

/// What one archived document is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKey {
    Day(NaiveDate),
    Activity(ActivityId),
}
