use std::fmt;

use serde::{Deserialize, Serialize};

/// The DI OAuth2 tokens, in python-garminconnect's `garmin_tokens.json`
/// format. Unknown keys in that file are ignored.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub di_token: String,
    pub di_refresh_token: String,
    pub di_client_id: String,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Tokens").finish_non_exhaustive()
    }
}
