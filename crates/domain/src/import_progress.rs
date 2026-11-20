use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportProgress {
    /// Days of a cursor: finished through `done_through`, `left` to go.
    Days { done_through: NaiveDate, left: u32 },
    /// Garmin gives no workout total, so only whether the history walk is over.
    Workouts { complete: bool },
}
