use std::fs::{self, File};
use std::path::Path;

use crate::StorageError;
use crate::sync_dir::sync_dir;

/// Atomically replaces `path` with what `write` produces: write a temp file,
/// fsync it, rename it over `path`, then fsync every directory from the
/// parent of `path` up to `root`, so new directories are durable too.
pub(crate) fn replace_file(
    root: &Path,
    path: &Path,
    write: impl FnOnce(&mut File) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let parent = path.parent().expect("file path has a parent directory");
    fs::create_dir_all(parent)?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let mut file = File::create(&temp)?;
    write(&mut file)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    for dir in parent.ancestors().take_while(|dir| dir.starts_with(root)) {
        sync_dir(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use tempfile::TempDir;

    use super::replace_file;

    #[test]
    fn creates_directories_and_leaves_no_temp_file() {
        let root = TempDir::new().expect("tempdir");
        let path = root.path().join("a").join("b").join("data");
        replace_file(root.path(), &path, |file| Ok(file.write_all(b"new")?)).expect("replace");
        assert_eq!(fs::read(&path).expect("read"), b"new");
        assert!(!path.with_extension("tmp").exists());
        assert_eq!(
            fs::read_dir(path.parent().expect("parent"))
                .expect("dir")
                .count(),
            1
        );
    }

    #[test]
    fn a_stale_temp_file_is_overwritten() {
        let root = TempDir::new().expect("tempdir");
        let path = root.path().join("data");
        fs::write(root.path().join("data.tmp"), b"garbage that is longer").expect("stale");
        replace_file(root.path(), &path, |file| Ok(file.write_all(b"new")?)).expect("replace");
        assert_eq!(fs::read(&path).expect("read"), b"new");
        assert!(!root.path().join("data.tmp").exists());
    }

    #[test]
    fn a_failed_write_keeps_the_old_file() {
        let root = TempDir::new().expect("tempdir");
        let path = root.path().join("data");
        fs::write(&path, b"old").expect("old");
        let outcome = replace_file(root.path(), &path, |_| {
            Err(std::io::Error::other("boom").into())
        });
        assert!(outcome.is_err());
        assert_eq!(fs::read(&path).expect("read"), b"old");
    }
}
