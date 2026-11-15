use chrono::NaiveDate;
use domain::Timestamp;

/// A calendar day to pull, and the timestamp its measurements carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthDay {
    pub date: NaiveDate,
    pub stamp: Timestamp,
}
