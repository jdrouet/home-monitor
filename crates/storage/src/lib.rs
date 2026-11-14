#[cfg_attr(not(test), expect(dead_code, reason = "wired up by ParquetStore"))]
mod reading_key;
mod storage_error;
#[cfg_attr(not(test), expect(dead_code, reason = "wired up by ParquetStore"))]
mod sync_dir;
#[cfg_attr(not(test), expect(dead_code, reason = "wired up by ParquetStore"))]
mod utc_date;
#[cfg_attr(not(test), expect(dead_code, reason = "wired up by ParquetStore"))]
mod wal;

pub use storage_error::StorageError;
