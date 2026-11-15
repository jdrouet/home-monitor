use std::path::{Path, PathBuf};

use chrono::Datelike;
use use_cases::{ArchiveKey, RawArchive};

use crate::StateFileError;
use crate::durable_dir::create_dirs_durably;
use crate::write_atomic::write_atomic;

/// Raw Garmin documents kept as files under one directory:
/// `daily/<YYYY>/<YYYY-MM-DD>/<file>` and `activities/<YYYY>/<id>/<file>`.
/// A refused document leaves an empty `<file>.denied` instead.
///
/// The root's parent is the mount point and is never created: without it the
/// archive is unavailable, so nothing lands on the container's own layer.
pub struct ArchiveDir {
    root: PathBuf,
}

impl ArchiveDir {
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, key: &ArchiveKey) -> PathBuf {
        match key {
            ArchiveKey::Day { date, .. } => self
                .root
                .join("daily")
                .join(format!("{:04}", date.year()))
                .join(date.to_string()),
            ArchiveKey::Activity { activity, .. } => self
                .root
                .join("activities")
                .join(format!("{:04}", activity.date.year()))
                .join(activity.id.to_string()),
        }
    }

    /// The directory the archive root lives in, when it exists.
    fn mount(&self) -> Result<&Path, StateFileError> {
        self.root
            .parent()
            .filter(|parent| parent.is_dir())
            .ok_or_else(|| StateFileError::ArchiveUnavailable {
                path: self.root.clone(),
            })
    }

    fn write(&self, key: &ArchiveKey, name: &str, bytes: &[u8]) -> Result<(), StateFileError> {
        let mount = self.mount()?;
        let dir = self.dir(key);
        let path = dir.join(name);
        create_dirs_durably(mount, &dir)
            .and_then(|()| write_atomic(&path, bytes))
            .map_err(|source| StateFileError::Write { path, source })
    }
}

fn file_name(key: &ArchiveKey) -> &'static str {
    match key {
        ArchiveKey::Day { file, .. } | ArchiveKey::Activity { file, .. } => file,
    }
}

fn denied_name(key: &ArchiveKey) -> String {
    format!("{}.denied", file_name(key))
}

impl RawArchive for ArchiveDir {
    type Error = StateFileError;

    fn contains(&self, key: &ArchiveKey) -> Result<bool, Self::Error> {
        self.mount()?;
        let dir = self.dir(key);
        let document = dir.join(file_name(key));
        document
            .try_exists()
            .and_then(|found| {
                if found {
                    Ok(true)
                } else {
                    dir.join(denied_name(key)).try_exists()
                }
            })
            .map_err(|source| StateFileError::Read {
                path: document,
                source,
            })
    }

    fn put(&self, key: &ArchiveKey, bytes: &[u8]) -> Result<(), Self::Error> {
        self.write(key, file_name(key), bytes)
    }

    fn put_denied(&self, key: &ArchiveKey) -> Result<(), Self::Error> {
        self.write(key, &denied_name(key), &[])
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::NaiveDate;
    use tempfile::TempDir;
    use use_cases::{ActivityId, ActivityRef, ArchiveKey, RawArchive};

    use crate::StateFileError;

    use super::ArchiveDir;

    fn day() -> ArchiveKey {
        ArchiveKey::Day {
            date: NaiveDate::from_ymd_opt(2026, 7, 14).expect("date"),
            file: "sleep.json",
        }
    }

    fn activity() -> ArchiveKey {
        ArchiveKey::Activity {
            activity: ActivityRef {
                id: ActivityId(987_654),
                date: NaiveDate::from_ymd_opt(2025, 12, 31).expect("date"),
            },
            file: "original.zip",
        }
    }

    fn archive_in(dir: &TempDir) -> ArchiveDir {
        ArchiveDir::new(dir.path().join("garmin"))
    }

    #[test]
    fn documents_are_laid_out_by_year_and_day_or_activity() {
        let dir = TempDir::new().expect("tempdir");
        let archive = archive_in(&dir);
        archive.put(&day(), b"day").expect("put day");
        archive.put(&activity(), b"zip").expect("put activity");
        let root = dir.path().join("garmin");
        assert_eq!(
            fs::read(root.join("daily/2026/2026-07-14/sleep.json")).expect("day"),
            b"day"
        );
        assert_eq!(
            fs::read(root.join("activities/2025/987654/original.zip")).expect("activity"),
            b"zip"
        );
    }

    #[test]
    fn a_second_put_replaces_the_document_and_leaves_no_temp_file() {
        let dir = TempDir::new().expect("tempdir");
        let archive = archive_in(&dir);
        archive.put(&day(), b"first").expect("put");
        archive.put(&day(), b"second").expect("put again");
        let folder = dir.path().join("garmin/daily/2026/2026-07-14");
        assert_eq!(
            fs::read(folder.join("sleep.json")).expect("read"),
            b"second"
        );
        assert_eq!(fs::read_dir(folder).expect("list").count(), 1);
    }

    #[test]
    fn no_bytes_make_an_empty_document() {
        let dir = TempDir::new().expect("tempdir");
        let archive = archive_in(&dir);
        archive.put(&day(), &[]).expect("put");
        let path = dir.path().join("garmin/daily/2026/2026-07-14/sleep.json");
        assert_eq!(fs::metadata(path).expect("metadata").len(), 0);
        assert!(archive.contains(&day()).expect("contains"));
    }

    #[test]
    fn a_document_is_contained_only_once_stored() {
        let dir = TempDir::new().expect("tempdir");
        let archive = archive_in(&dir);
        assert!(!archive.contains(&day()).expect("contains"));
        archive.put(&day(), b"x").expect("put");
        assert!(archive.contains(&day()).expect("contains"));
        assert!(!archive.contains(&activity()).expect("contains"));
    }

    #[test]
    fn a_denied_document_leaves_a_marker_and_counts_as_contained() {
        let dir = TempDir::new().expect("tempdir");
        let archive = archive_in(&dir);
        archive.put_denied(&day()).expect("denied");
        let folder = dir.path().join("garmin/daily/2026/2026-07-14");
        assert_eq!(
            fs::metadata(folder.join("sleep.json.denied"))
                .expect("marker")
                .len(),
            0
        );
        assert!(!folder.join("sleep.json").exists());
        assert!(archive.contains(&day()).expect("contains"));
    }

    #[test]
    fn a_missing_mount_point_is_unavailable_and_nothing_is_created() {
        let dir = TempDir::new().expect("tempdir");
        let archive = ArchiveDir::new(dir.path().join("absent/garmin"));
        assert!(matches!(
            archive.contains(&day()),
            Err(StateFileError::ArchiveUnavailable { .. })
        ));
        assert!(matches!(
            archive.put(&day(), b"x"),
            Err(StateFileError::ArchiveUnavailable { .. })
        ));
        assert!(matches!(
            archive.put_denied(&day()),
            Err(StateFileError::ArchiveUnavailable { .. })
        ));
        assert!(!dir.path().join("absent").exists());
    }

    #[test]
    fn a_blocked_directory_is_a_write_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("garmin"), b"file").expect("block");
        let archive = archive_in(&dir);
        assert!(matches!(
            archive.put(&day(), b"x"),
            Err(StateFileError::Write { .. })
        ));
    }

    #[test]
    fn an_unreadable_location_is_a_read_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("garmin"), b"file").expect("block");
        let archive = archive_in(&dir);
        assert!(matches!(
            archive.contains(&day()),
            Err(StateFileError::Read { .. })
        ));
    }
}
