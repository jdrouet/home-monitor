use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

/// The content of `path`, or `None` when the file does not exist.
pub(crate) fn read_if_present(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(source),
    }
}
