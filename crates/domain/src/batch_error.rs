#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("invalid batch JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported batch schema version {0}")]
    UnsupportedVersion(u32),
}
