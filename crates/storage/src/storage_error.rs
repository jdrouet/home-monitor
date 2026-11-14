use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage I/O failed")]
    Io(#[from] io::Error),
    #[error("corrupt log segment {} at line {line}", path.display())]
    CorruptLog {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}
