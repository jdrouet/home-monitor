use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::DomainError;

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

impl FromStr for SourceKind {
    type Err = DomainError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "ble" => Ok(Self::Ble),
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
        assert_eq!(SourceKind::Ble.as_str().parse(), Ok(SourceKind::Ble));
    }

    #[test]
    fn rejects_an_unknown_kind() {
        assert_eq!(
            "zigbee".parse::<SourceKind>(),
            Err(DomainError::UnknownSourceKind("zigbee".to_owned()))
        );
    }
}
