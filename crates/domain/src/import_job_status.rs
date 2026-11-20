use serde::{Deserialize, Serialize};

use crate::{ImportAttempt, ImportEta, ImportProgress, Timestamp};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportJobStatus {
    /// First try, then retries, oldest first.
    pub attempts: Vec<ImportAttempt>,
    /// The scheduled pull a job that gave up waits for; unset otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<Timestamp>,
    /// The instant of a planned retry; never set together with `retry_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ImportProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta: Option<ImportEta>,
}

impl ImportJobStatus {
    /// Retries that ran in the current window: every attempt after the first.
    pub fn retries(&self) -> usize {
        self.attempts.len().saturating_sub(1)
    }
}
