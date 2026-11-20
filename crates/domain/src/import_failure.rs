use serde::{Deserialize, Serialize};

/// Why an import stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFailure {
    LoginRejected,
    Throttled,
    Unreachable,
    GarminDown,
    BadAnswer,
    LoginFileUnreadable,
    ArchiveUnavailable,
    ProgressUnreadable,
    ProgressNotSaved,
    PublishFailed,
}
