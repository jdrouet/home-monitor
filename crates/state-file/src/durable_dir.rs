use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::path::Path;

/// Creates the missing directories from `base` down to `dir`, one level at a
/// time, syncing the parent of each new one so a crash cannot lose it. `base`
/// must exist and `dir` must lie under it.
pub(crate) fn create_dirs_durably(base: &Path, dir: &Path) -> io::Result<()> {
    let relative = dir
        .strip_prefix(base)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "directory is outside its base"))?;
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let parent = current.clone();
        current.push(component);
        match fs::create_dir(&current) {
            Ok(()) => File::open(&parent)?.sync_all()?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use tempfile::TempDir;

    use super::create_dirs_durably;

    #[test]
    fn missing_levels_are_created_and_existing_ones_are_kept() {
        let base = TempDir::new().expect("tempdir");
        let dir = base.path().join("a/b/c");
        create_dirs_durably(base.path(), &dir).expect("create");
        assert!(dir.is_dir());
        create_dirs_durably(base.path(), &dir).expect("again");
    }

    #[test]
    fn a_missing_base_is_not_created() {
        let base = TempDir::new().expect("tempdir");
        let missing = base.path().join("absent");
        let error = create_dirs_durably(&missing, &missing.join("a")).expect_err("no base");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(!missing.exists());
    }

    #[test]
    fn a_directory_outside_the_base_is_refused() {
        let base = TempDir::new().expect("tempdir");
        let other = TempDir::new().expect("tempdir");
        let error = create_dirs_durably(base.path(), other.path()).expect_err("outside");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn a_file_in_the_way_is_an_error() {
        let base = TempDir::new().expect("tempdir");
        std::fs::write(base.path().join("a"), b"").expect("file");
        let error = create_dirs_durably(base.path(), &base.path().join("a/b")).expect_err("file");
        assert_eq!(error.kind(), ErrorKind::NotADirectory);
    }
}
