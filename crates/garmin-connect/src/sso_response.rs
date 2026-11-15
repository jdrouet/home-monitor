use reqwest::StatusCode;
use serde::Deserialize;

use crate::GarminError;

const DEFAULT_MFA_METHOD: &str = "email";

/// What a sign-in or MFA answer asks for next. The ticket is a credential, so
/// this type has no `Debug`.
pub enum SsoOutcome {
    Ticket(String),
    MfaRequired { method: String },
}

#[derive(Deserialize)]
struct SsoResponse {
    #[serde(rename = "responseStatus")]
    status: Option<ResponseStatus>,
    #[serde(rename = "serviceTicketId")]
    ticket: Option<String>,
    #[serde(rename = "customerMfaInfo")]
    mfa_info: Option<MfaInfo>,
    error: Option<SsoError>,
}

#[derive(Deserialize)]
struct ResponseStatus {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct MfaInfo {
    #[serde(rename = "mfaLastMethodUsed")]
    method: Option<String>,
}

#[derive(Deserialize)]
struct SsoError {
    #[serde(rename = "status-code")]
    status_code: Option<String>,
}

impl SsoOutcome {
    /// Reads a sign-in or MFA answer. Errors name the response type only.
    pub fn parse(bytes: &[u8]) -> Result<Self, GarminError> {
        let response: SsoResponse = serde_json::from_slice(bytes)
            .map_err(|_| GarminError::Unexpected("not a JSON response".to_owned()))?;
        if response.is_rate_limited() {
            return Err(GarminError::Blocked(StatusCode::TOO_MANY_REQUESTS));
        }
        let kind = response
            .status
            .and_then(|status| status.kind)
            .unwrap_or_else(|| "missing".to_owned());
        match kind.as_str() {
            "SUCCESSFUL" => response
                .ticket
                .map(Self::Ticket)
                .ok_or(GarminError::Unexpected(kind)),
            "MFA_REQUIRED" => Ok(Self::MfaRequired {
                method: response
                    .mfa_info
                    .and_then(|info| info.method)
                    .unwrap_or_else(|| DEFAULT_MFA_METHOD.to_owned()),
            }),
            "INVALID_USERNAME_PASSWORD" => Err(GarminError::InvalidCredentials),
            _ => Err(GarminError::Unexpected(kind)),
        }
    }
}

impl SsoResponse {
    fn is_rate_limited(&self) -> bool {
        self.error
            .as_ref()
            .and_then(|error| error.status_code.as_deref())
            == Some("429")
    }
}

#[cfg(test)]
mod tests {
    use crate::GarminError;

    use super::SsoOutcome;

    fn parse(text: &str) -> Result<SsoOutcome, GarminError> {
        SsoOutcome::parse(text.as_bytes())
    }

    #[test]
    fn success_gives_the_ticket() {
        let outcome = parse(include_str!("../fixtures/sso_success.json"));
        assert!(matches!(outcome, Ok(SsoOutcome::Ticket(ticket)) if ticket == "ST-TESTTICKET-sso"));
    }

    #[test]
    fn mfa_required_gives_the_last_method_or_email() {
        let outcome = parse(include_str!("../fixtures/sso_mfa_required.json"));
        assert!(matches!(outcome, Ok(SsoOutcome::MfaRequired { method }) if method == "email"));
        let bare = parse(r#"{"responseStatus":{"type":"MFA_REQUIRED"}}"#);
        assert!(matches!(bare, Ok(SsoOutcome::MfaRequired { method }) if method == "email"));
        let sms = parse(
            r#"{"responseStatus":{"type":"MFA_REQUIRED"},"customerMfaInfo":{"mfaLastMethodUsed":"sms"}}"#,
        );
        assert!(matches!(sms, Ok(SsoOutcome::MfaRequired { method }) if method == "sms"));
    }

    #[test]
    fn invalid_credentials_are_reported() {
        let outcome = parse(include_str!("../fixtures/sso_invalid.json"));
        assert!(matches!(outcome, Err(GarminError::InvalidCredentials)));
    }

    #[test]
    fn a_json_rate_limit_is_a_block() {
        let outcome = parse(r#"{"error":{"status-code":"429","message":"slow down"}}"#);
        assert!(matches!(outcome, Err(GarminError::Blocked(status)) if status.as_u16() == 429));
    }

    #[test]
    fn other_types_are_unexpected_and_named() {
        for (text, name) in [
            (
                r#"{"responseStatus":{"type":"CAPTCHA_REQUIRED"}}"#,
                "CAPTCHA_REQUIRED",
            ),
            (r#"{"responseStatus":{"type":"SUCCESSFUL"}}"#, "SUCCESSFUL"),
            (r"{}", "missing"),
        ] {
            assert!(matches!(parse(text), Err(GarminError::Unexpected(kind)) if kind == name));
        }
    }

    #[test]
    fn non_json_is_unexpected_without_echoing_the_body() {
        let Err(error) = parse("<html>secret-page</html>") else {
            panic!("not JSON");
        };
        assert!(matches!(error, GarminError::Unexpected(_)));
        assert!(!format!("{error:?}{error}").contains("secret-page"));
    }
}
