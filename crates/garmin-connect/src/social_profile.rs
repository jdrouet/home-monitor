use serde::Deserialize;

use crate::GarminError;

#[derive(Deserialize)]
pub struct SocialProfile {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

impl SocialProfile {
    /// The account's display name, which keys the data endpoints.
    pub fn display_name(self) -> Result<String, GarminError> {
        self.display_name
            .filter(|name| !name.trim().is_empty())
            .ok_or(GarminError::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use crate::GarminError;

    use super::SocialProfile;

    fn parse(text: &str) -> SocialProfile {
        serde_json::from_str(text).expect("valid profile")
    }

    #[test]
    fn the_fixture_has_a_display_name() {
        let profile = parse(include_str!("../fixtures/social_profile.json"));
        assert_eq!(profile.display_name().expect("name"), "Jane Doe");
    }

    #[test]
    fn missing_and_blank_names_are_malformed() {
        for text in [r"{}", r#"{"displayName":"  "}"#, r#"{"displayName":null}"#] {
            assert!(matches!(
                parse(text).display_name(),
                Err(GarminError::Malformed)
            ));
        }
    }
}
