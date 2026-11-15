use chrono::NaiveDate;

use crate::ActivityId;

/// An activity in Garmin's list: its id and the local day it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActivityRef {
    pub id: ActivityId,
    pub date: NaiveDate,
}
