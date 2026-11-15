use std::io;
use std::path::PathBuf;

use reqwest::StatusCode;

/// Failures of the Garmin adapter. Messages never carry tokens, URLs or
/// response bodies.
#[derive(Debug, thiserror::Error)]
pub enum GarminError {
    #[error("cannot access the token file {}", path.display())]
    TokenFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the token file {} is not valid", path.display())]
    InvalidTokenFile { path: PathBuf },
    #[error("request to Garmin failed")]
    Request(#[source] reqwest::Error),
    #[error("Garmin refused to refresh the tokens (status {0})")]
    Refresh(StatusCode),
    #[error("Garmin rejected the session (status {0})")]
    SessionRejected(StatusCode),
    #[error("Garmin answered with status {0}")]
    Status(StatusCode),
    #[error("Garmin sent a response that cannot be used")]
    Malformed,
}

impl GarminError {
    /// Wraps a transport error without the URL it carries.
    pub(crate) fn request(error: reqwest::Error) -> Self {
        Self::Request(error.without_url())
    }
}
