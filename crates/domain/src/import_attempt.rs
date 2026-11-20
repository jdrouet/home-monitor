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
    #[serde(default)]
    pub counts: ImportCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Done,
    /// The job ran out of time and resumes after the next pull.
    Paused,
    /// The activity catch-up read its page limit with time to spare, so later
    /// jobs still ran in the same window.
    Capped,
    Failed(ImportFailure),
}

#[cfg(test)]
mod tests {
    use super::AttemptOutcome;

    #[test]
    fn a_capped_outcome_is_its_own_word_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&AttemptOutcome::Capped).unwrap(),
            r#""capped""#
        );
        assert_eq!(
            serde_json::from_str::<AttemptOutcome>(r#""paused""#).unwrap(),
            AttemptOutcome::Paused
        );
    }
}
