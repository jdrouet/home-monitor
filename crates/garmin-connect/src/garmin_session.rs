use std::fmt;

use crate::Tokens;

/// Fresh tokens and the display name that keys the data endpoints.
pub struct GarminSession {
    pub(crate) tokens: Tokens,
    pub(crate) display_name: String,
}

impl fmt::Debug for GarminSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GarminSession")
            .finish_non_exhaustive()
    }
}
