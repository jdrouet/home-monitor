use serde::{Deserialize, Serialize};

/// Which side of its allowed range a temperature is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Deviation {
    TooCold,
    TooHot,
}
