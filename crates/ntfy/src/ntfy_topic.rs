use std::fmt;
use std::str::FromStr;

use crate::NtfyError;

const MAX_LEN: usize = 64;

/// The secret name of a ntfy topic. It is never printed.
pub struct NtfyTopic(String);

impl NtfyTopic {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for NtfyTopic {
    type Err = NtfyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let valid = (1..=MAX_LEN).contains(&text.len())
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(NtfyError::InvalidTopic)
        }
    }
}

impl fmt::Debug for NtfyTopic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NtfyTopic(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use crate::NtfyError;

    use super::NtfyTopic;

    #[test]
    fn letters_digits_dash_and_underscore_are_valid() {
        let topic: NtfyTopic = "Home_monitor-42".parse().expect("valid topic");
        assert_eq!(topic.as_str(), "Home_monitor-42");
        assert!("a".repeat(64).parse::<NtfyTopic>().is_ok());
    }

    #[test]
    fn invalid_topics_are_rejected() {
        for text in ["", &"a".repeat(65), "a/b", "a b", "é", "a?b"] {
            assert!(matches!(
                text.parse::<NtfyTopic>(),
                Err(NtfyError::InvalidTopic)
            ));
        }
    }

    #[test]
    fn debug_hides_the_topic() {
        let topic: NtfyTopic = "secretname".parse().expect("valid topic");
        assert!(!format!("{topic:?}").contains("secretname"));
    }
}
