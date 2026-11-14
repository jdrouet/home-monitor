use std::io;
use std::path::PathBuf;

use datafusion::arrow::error::ArrowError;
use datafusion::parquet::errors::ParquetError;
use domain::BatchError;

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
    #[error("cannot read or write a day file")]
    Parquet(#[from] ParquetError),
    #[error("cannot build a record batch")]
    Arrow(#[from] ArrowError),
    #[error("corrupt latest-values snapshot")]
    CorruptSnapshot(#[from] BatchError),
    #[error("invalid day file column: {0}")]
    InvalidColumn(String),
}
