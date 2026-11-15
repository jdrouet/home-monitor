use serde::Deserialize;

use crate::{GarminError, Tokens};

/// The DI token endpoint's answer. Only the two tokens are read.
#[derive(Deserialize)]
pub struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

impl TokenResponse {
    /// The tokens to store; `previous_refresh` stands in when Garmin sends no
    /// refresh token or a blank one. A blank access token is malformed.
    pub fn into_tokens(
        self,
        client_id: String,
        previous_refresh: Option<&str>,
    ) -> Result<Tokens, GarminError> {
        if self.access_token.trim().is_empty() {
            return Err(GarminError::Malformed);
        }
        let di_refresh_token = self
            .refresh_token
            .filter(|token| !token.trim().is_empty())
            .or_else(|| previous_refresh.map(str::to_owned))
            .ok_or(GarminError::Malformed)?;
        Ok(Tokens {
            di_token: self.access_token,
            di_refresh_token,
            di_client_id: client_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::GarminError;

    use super::TokenResponse;

    const FIXTURE: &str = include_str!("../fixtures/di_token.json");

    fn parse(text: &str) -> TokenResponse {
        serde_json::from_str(text).expect("valid response")
    }

    #[test]
    fn the_fixture_gives_both_tokens() {
        let tokens = parse(FIXTURE)
            .into_tokens("client".to_owned(), Some("old"))
            .expect("tokens");
        assert_eq!(tokens.di_token, "new-access-token");
        assert_eq!(tokens.di_refresh_token, "new-refresh-token");
        assert_eq!(tokens.di_client_id, "client");
    }

    #[test]
    fn a_missing_refresh_token_keeps_the_previous_one() {
        let tokens = parse(r#"{"access_token":"a"}"#)
            .into_tokens("client".to_owned(), Some("old"))
            .expect("tokens");
        assert_eq!(tokens.di_refresh_token, "old");
    }

    #[test]
    fn a_blank_refresh_token_keeps_the_previous_one() {
        for blank in ["", "  "] {
            let tokens = parse(&format!(
                r#"{{"access_token":"a","refresh_token":"{blank}"}}"#
            ))
            .into_tokens("client".to_owned(), Some("old"))
            .expect("tokens");
            assert_eq!(tokens.di_refresh_token, "old");
        }
    }

    #[test]
    fn a_blank_refresh_token_without_a_previous_one_is_malformed() {
        let result = parse(r#"{"access_token":"a","refresh_token":" "}"#)
            .into_tokens("client".to_owned(), None);
        assert!(matches!(result, Err(GarminError::Malformed)));
    }

    #[test]
    fn a_blank_access_token_is_malformed() {
        for blank in ["", "  "] {
            let result = parse(&format!(
                r#"{{"access_token":"{blank}","refresh_token":"r"}}"#
            ))
            .into_tokens("client".to_owned(), Some("old"));
            assert!(matches!(result, Err(GarminError::Malformed)));
        }
    }

    #[test]
    fn no_refresh_token_at_all_is_malformed() {
        let result = parse(r#"{"access_token":"a"}"#).into_tokens("client".to_owned(), None);
        assert!(matches!(result, Err(GarminError::Malformed)));
    }

    #[test]
    fn a_response_without_an_access_token_does_not_parse() {
        assert!(serde_json::from_str::<TokenResponse>(r#"{"refresh_token":"r"}"#).is_err());
    }
}
