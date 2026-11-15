#[derive(Debug, thiserror::Error)]
pub enum AlertReportError {
    #[error("invalid alert report JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported alert report schema version {0}")]
    UnsupportedVersion(u32),
}
