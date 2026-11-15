use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use use_cases::{ActivityCursor, ActivityCursorStore};

use crate::StateFileError;
use crate::read_if_present::read_if_present;
use crate::write_atomic::write_atomic;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Wire {
    version: u32,
    start: u32,
    done: bool,
    catch_up: Option<u32>,
    #[serde(default)]
    boundary: Option<u64>,
    #[serde(default)]
    pending: Option<u64>,
}

/// Activity import cursor kept in one JSON file, replaced atomically on every
/// save.
pub struct ActivityCursorFile {
    path: PathBuf,
}

impl ActivityCursorFile {
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl ActivityCursorStore for ActivityCursorFile {
    type Error = StateFileError;

    fn load(&self) -> Result<Option<ActivityCursor>, Self::Error> {
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
        Ok(Some(ActivityCursor {
            start: wire.start,
            done: wire.done,
            catch_up: wire.catch_up,
            boundary: wire.boundary,
            pending: wire.pending,
        }))
    }

    fn save(&self, cursor: &ActivityCursor) -> Result<(), Self::Error> {
        let wire = Wire {
            version: VERSION,
            start: cursor.start,
            done: cursor.done,
            catch_up: cursor.catch_up,
            boundary: cursor.boundary,
            pending: cursor.pending,
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

    use tempfile::TempDir;
    use use_cases::{ActivityCursor, ActivityCursorStore};

    use crate::StateFileError;

    use super::ActivityCursorFile;

    fn file_in(dir: &TempDir) -> ActivityCursorFile {
        ActivityCursorFile::new(dir.path().join("garmin_activities.json"))
    }

    #[test]
    fn a_missing_file_is_no_cursor() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(file_in(&dir).load().expect("load"), None);
    }

    #[test]
    fn a_saved_cursor_is_loaded_back_with_and_without_a_catch_up() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        for catch_up in [None, Some(15)] {
            let cursor = ActivityCursor {
                start: 40,
                done: true,
                catch_up,
                boundary: Some(77),
                pending: catch_up.map(|_| 99),
            };
            file.save(&cursor).expect("save");
            assert_eq!(file.load().expect("load"), Some(cursor));
        }
        let json = fs::read_to_string(dir.path().join("garmin_activities.json")).expect("read");
        assert_eq!(
            json,
            r#"{"version":1,"start":40,"done":true,"catch_up":15,"boundary":77,"pending":99}"#
        );
    }

    #[test]
    fn a_cleared_catch_up_is_saved_as_null() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&ActivityCursor::fresh()).expect("save");
        let json = fs::read_to_string(dir.path().join("garmin_activities.json")).expect("read");
        assert_eq!(
            json,
            r#"{"version":1,"start":0,"done":false,"catch_up":null,"boundary":null,"pending":null}"#
        );
    }

    #[test]
    fn a_file_without_the_boundary_fields_loads_with_none() {
        let dir = TempDir::new().expect("tempdir");
        let json = r#"{"version":1,"start":5,"done":true,"catch_up":null}"#;
        fs::write(dir.path().join("garmin_activities.json"), json).expect("write");
        let cursor = file_in(&dir).load().expect("load").expect("cursor");
        assert_eq!((cursor.boundary, cursor.pending), (None, None));
    }

    #[test]
    fn another_version_is_rejected() {
        let dir = TempDir::new().expect("tempdir");
        let json = r#"{"version":2,"start":0,"done":false,"catch_up":null}"#;
        fs::write(dir.path().join("garmin_activities.json"), json).expect("write");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::UnsupportedVersion { version: 2, .. })
        ));
    }

    #[test]
    fn garbage_is_a_parse_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("garmin_activities.json"), b"not json").expect("write");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::Parse { .. })
        ));
    }

    #[test]
    fn an_unreadable_path_is_a_read_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir(dir.path().join("garmin_activities.json")).expect("dir");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::Read { .. })
        ));
    }

    #[test]
    fn a_failed_save_is_a_write_error_and_keeps_the_old_file() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&ActivityCursor::fresh()).expect("save");
        fs::create_dir(dir.path().join("garmin_activities.json.tmp")).expect("block");
        let outcome = file.save(&ActivityCursor {
            start: 5,
            ..ActivityCursor::fresh()
        });
        assert!(matches!(outcome, Err(StateFileError::Write { .. })));
        assert_eq!(file.load().expect("load"), Some(ActivityCursor::fresh()));
    }
}
