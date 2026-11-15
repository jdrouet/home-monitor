use serde::{Deserialize, Serialize};

use crate::Deviation;

/// What an alert was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Offline,
    TooCold,
    TooHot,
    GarminLogin,
}

impl From<Deviation> for AlertKind {
    fn from(deviation: Deviation) -> Self {
        match deviation {
            Deviation::TooCold => Self::TooCold,
            Deviation::TooHot => Self::TooHot,
        }
    }
}
