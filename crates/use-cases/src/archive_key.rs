use chrono::NaiveDate;

use crate::ActivityRef;

/// Where one raw document lives in the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKey {
    Day {
        date: NaiveDate,
        file: &'static str,
    },
    Activity {
        activity: ActivityRef,
        file: &'static str,
    },
}
