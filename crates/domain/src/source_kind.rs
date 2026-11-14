use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Ble,
}

impl SourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ble => "ble",
        }
    }
}
