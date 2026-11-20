use serde::{Deserialize, Serialize};

use crate::{ImportAttempt, ImportEta, ImportProgress, Timestamp};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportJobStatus {
    /// First try, then retries, oldest first.
    pub attempts: Vec<ImportAttempt>,
    /// The scheduled pull a failed job waits for; unset when it will not wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ImportProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta: Option<ImportEta>,
}
