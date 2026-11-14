use std::fmt;

use serde::Deserialize;

use crate::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct RoomName(String);

impl RoomName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RoomName {
    type Error = DomainError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if name.trim().is_empty() {
            return Err(DomainError::EmptyRoomName);
        }
        Ok(Self(name))
    }
}

impl fmt::Display for RoomName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use crate::DomainError;

    use super::RoomName;

    #[test]
    fn empty_and_blank_names_are_rejected() {
        assert_eq!(
            RoomName::try_from(String::new()),
            Err(DomainError::EmptyRoomName)
        );
        assert_eq!(
            RoomName::try_from("  ".to_owned()),
            Err(DomainError::EmptyRoomName)
        );
    }

    #[test]
    fn name_is_kept_verbatim() {
        let name = RoomName::try_from("living room".to_owned()).expect("valid name");
        assert_eq!(name.as_str(), "living room");
        assert_eq!(name.to_string(), "living room");
    }
}
