use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Ble,
    Weather,
}

impl SourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ble => "ble",
            Self::Weather => "weather",
        }
    }
}

impl FromStr for SourceKind {
    type Err = DomainError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "ble" => Ok(Self::Ble),
            "weather" => Ok(Self::Weather),
            other => Err(DomainError::UnknownSourceKind(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SourceKind;
    use crate::DomainError;

    #[test]
    fn parses_the_name_it_prints() {
        for kind in [SourceKind::Ble, SourceKind::Weather] {
            assert_eq!(kind.as_str().parse(), Ok(kind));
        }
    }

    #[test]
    fn rejects_an_unknown_kind() {
        assert_eq!(
            "zigbee".parse::<SourceKind>(),
            Err(DomainError::UnknownSourceKind("zigbee".to_owned()))
        );
    }
}
