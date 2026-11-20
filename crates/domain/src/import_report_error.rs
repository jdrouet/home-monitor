#[derive(Debug, thiserror::Error)]
pub enum ImportReportError {
    #[error("invalid import report JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported import report schema version {0}")]
    UnsupportedVersion(u32),
}
