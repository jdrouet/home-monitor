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

impl ImportJobStatus {
    pub fn latest(&self) -> Option<&ImportAttempt> {
        self.attempts.last()
    }
}

#[cfg(test)]
mod tests {
    use crate::{ImportAttempt, Timestamp};

    use super::ImportJobStatus;

    fn attempt(started: i64) -> ImportAttempt {
        ImportAttempt {
            started: Timestamp::from_micros(started),
            ended: None,
        }
    }

    #[test]
    fn latest_is_none_without_attempts() {
        assert_eq!(ImportJobStatus::default().latest(), None);
    }

    #[test]
    fn latest_is_the_last_attempt() {
        let status = ImportJobStatus {
            attempts: vec![attempt(1), attempt(2)],
            ..ImportJobStatus::default()
        };
        assert_eq!(status.latest(), Some(&attempt(2)));
    }
}
