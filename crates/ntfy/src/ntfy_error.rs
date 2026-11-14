#[derive(Debug, thiserror::Error)]
pub enum NtfyError {
    #[error("invalid ntfy url {0}")]
    InvalidUrl(String),
    #[error("ntfy topic must be 1 to 64 letters, digits, '-' or '_'")]
    InvalidTopic,
    #[error("ntfy request failed: {0}")]
    Send(reqwest::Error),
}
