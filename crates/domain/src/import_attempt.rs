use serde::{Deserialize, Serialize};

use crate::{ImportCounts, ImportFailure, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportAttempt {
    pub started: Timestamp,
    /// `None` while the attempt runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<AttemptEnd>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptEnd {
    pub at: Timestamp,
    pub outcome: AttemptOutcome,
    #[serde(default, skip_serializing_if = "ImportCounts::is_empty")]
    pub counts: ImportCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Done,
    Paused,
    Failed(ImportFailure),
}
