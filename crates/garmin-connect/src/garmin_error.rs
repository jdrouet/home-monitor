use std::io;
use std::path::PathBuf;

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
}
