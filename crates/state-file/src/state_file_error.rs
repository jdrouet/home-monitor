use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StateFileError {
    #[error("cannot read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot parse {}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{} has unsupported version {version}", path.display())]
    UnsupportedVersion { path: PathBuf, version: u32 },
    #[error("cannot write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
