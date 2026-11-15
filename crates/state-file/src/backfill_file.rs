use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use use_cases::{BackfillCursor, BackfillStore};

use crate::StateFileError;
use crate::read_if_present::read_if_present;
use crate::write_atomic::write_atomic;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Wire {
    version: u32,
    next: NaiveDate,
    last: NaiveDate,
}

/// Health backfill cursor kept in one JSON file, replaced atomically on every
/// save.
pub struct BackfillFile {
    path: PathBuf,
}

impl BackfillFile {
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl BackfillStore for BackfillFile {
    type Error = StateFileError;

    fn load(&self) -> Result<Option<BackfillCursor>, Self::Error> {
        let bytes = read_if_present(&self.path).map_err(|source| StateFileError::Read {
            path: self.path.clone(),
            source,
        })?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let wire: Wire =
            serde_json::from_slice(&bytes).map_err(|source| StateFileError::Parse {
                path: self.path.clone(),
                source,
            })?;
        if wire.version != VERSION {
            return Err(StateFileError::UnsupportedVersion {
                path: self.path.clone(),
                version: wire.version,
            });
        }
        Ok(Some(BackfillCursor {
            next: wire.next,
            last: wire.last,
        }))
    }

    fn save(&self, cursor: &BackfillCursor) -> Result<(), Self::Error> {
        let wire = Wire {
            version: VERSION,
            next: cursor.next,
            last: cursor.last,
        };
        serde_json::to_vec(&wire)
            .map_err(std::io::Error::from)
            .and_then(|bytes| write_atomic(&self.path, &bytes))
            .map_err(|source| StateFileError::Write {
                path: self.path.clone(),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::NaiveDate;
    use tempfile::TempDir;
    use use_cases::{BackfillCursor, BackfillStore};

    use crate::StateFileError;

    use super::BackfillFile;

    fn cursor() -> BackfillCursor {
        BackfillCursor {
            next: NaiveDate::from_ymd_opt(2025, 11, 15).expect("date"),
            last: NaiveDate::from_ymd_opt(2026, 11, 13).expect("date"),
        }
    }

    fn file_in(dir: &TempDir) -> BackfillFile {
        BackfillFile::new(dir.path().join("garmin_backfill.json"))
    }

    #[test]
    fn a_missing_file_is_no_cursor() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(file_in(&dir).load().expect("load"), None);
    }

    #[test]
    fn a_saved_cursor_is_loaded_back() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&cursor()).expect("save");
        assert_eq!(file.load().expect("load"), Some(cursor()));
        let json = fs::read_to_string(dir.path().join("garmin_backfill.json")).expect("read");
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"next\":\"2025-11-15\""));
    }

    #[test]
    fn another_version_is_rejected() {
        let dir = TempDir::new().expect("tempdir");
        let json = r#"{"version":2,"next":"2025-11-15","last":"2026-11-13"}"#;
        fs::write(dir.path().join("garmin_backfill.json"), json).expect("write");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::UnsupportedVersion { version: 2, .. })
        ));
    }

    #[test]
    fn garbage_is_a_parse_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("garmin_backfill.json"), b"not json").expect("write");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::Parse { .. })
        ));
    }

    #[test]
    fn an_unreadable_path_is_a_read_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir(dir.path().join("garmin_backfill.json")).expect("dir");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::Read { .. })
        ));
    }

    #[test]
    fn saving_leaves_no_temp_file() {
        let dir = TempDir::new().expect("tempdir");
        file_in(&dir).save(&cursor()).expect("save");
        assert!(!dir.path().join("garmin_backfill.json.tmp").exists());
    }

    #[test]
    fn a_failed_save_keeps_the_old_file() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&cursor()).expect("save");
        let before = fs::read(dir.path().join("garmin_backfill.json")).expect("read");
        fs::create_dir(dir.path().join("garmin_backfill.json.tmp")).expect("block the temp file");
        let outcome = file.save(&cursor().advanced());
        assert!(matches!(outcome, Err(StateFileError::Write { .. })));
        assert_eq!(
            fs::read(dir.path().join("garmin_backfill.json")).expect("read"),
            before
        );
    }
}
