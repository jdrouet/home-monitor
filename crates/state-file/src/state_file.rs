use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use domain::AlertState;
use serde::{Deserialize, Serialize};
use use_cases::AlertStateStore;

use crate::StateFileError;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Wire {
    version: u32,
    #[serde(flatten)]
    state: AlertState,
}

/// Alert state kept in one JSON file, replaced atomically on every save.
pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn write(&self, state: &AlertState) -> io::Result<()> {
        let wire = Wire {
            version: VERSION,
            state: state.clone(),
        };
        let mut temp = self.path.as_os_str().to_owned();
        temp.push(".tmp");
        let mut file = File::create(&temp)?;
        file.write_all(&serde_json::to_vec(&wire)?)?;
        file.sync_all()?;
        fs::rename(&temp, &self.path)?;
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        File::open(parent)?.sync_all()
    }
}

impl AlertStateStore for StateFile {
    type Error = StateFileError;

    fn load(&self) -> Result<AlertState, Self::Error> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Ok(AlertState::default());
            }
            Err(source) => {
                return Err(StateFileError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
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
        Ok(wire.state)
    }

    fn save(&self, state: &AlertState) -> Result<(), Self::Error> {
        self.write(state).map_err(|source| StateFileError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use domain::{AlertState, Deviation, OfflineAlert, SourceId, TemperatureAlert, Timestamp};
    use tempfile::TempDir;
    use use_cases::AlertStateStore;

    use crate::StateFileError;

    use super::StateFile;

    fn state() -> AlertState {
        let mut state = AlertState::default();
        state.offline.insert(
            SourceId::from("A4:C1:38:00:00:01".to_owned()),
            OfflineAlert {
                last_seen: Timestamp::from_micros(10),
                notified_at: Timestamp::from_micros(20),
            },
        );
        state.temperature.insert(
            SourceId::from("A4:C1:38:00:00:02".to_owned()),
            TemperatureAlert {
                deviation: Deviation::TooHot,
                celsius: 21.3,
                notified_at: Timestamp::from_micros(30),
            },
        );
        state
    }

    fn file_in(dir: &TempDir) -> StateFile {
        StateFile::new(dir.path().join("alerts.json"))
    }

    fn temp_path(dir: &Path) -> std::path::PathBuf {
        dir.join("alerts.json.tmp")
    }

    #[test]
    fn a_missing_file_is_an_empty_state() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(file_in(&dir).load().expect("load"), AlertState::default());
    }

    #[test]
    fn a_saved_state_is_loaded_back() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&state()).expect("save");
        assert_eq!(file.load().expect("load"), state());
        let json = fs::read_to_string(dir.path().join("alerts.json")).expect("read");
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"deviation\":\"too_hot\""));
    }

    #[test]
    fn saving_leaves_no_temp_file_and_replaces_a_stale_one() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(temp_path(dir.path()), b"stale garbage that is longer").expect("stale");
        file_in(&dir).save(&state()).expect("save");
        assert!(!temp_path(dir.path()).exists());
    }

    #[test]
    fn garbage_is_a_parse_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("alerts.json"), b"not json").expect("write");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::Parse { .. })
        ));
    }

    #[test]
    fn another_version_is_rejected() {
        let dir = TempDir::new().expect("tempdir");
        let json = r#"{"version":2,"offline":{},"temperature":{}}"#;
        fs::write(dir.path().join("alerts.json"), json).expect("write");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::UnsupportedVersion { version: 2, .. })
        ));
    }

    #[test]
    fn an_unreadable_path_is_a_read_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir(dir.path().join("alerts.json")).expect("dir");
        assert!(matches!(
            file_in(&dir).load(),
            Err(StateFileError::Read { .. })
        ));
    }

    #[test]
    fn a_failed_save_keeps_the_old_file() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&state()).expect("save");
        let before = fs::read(dir.path().join("alerts.json")).expect("read");
        fs::create_dir(temp_path(dir.path())).expect("block the temp file");
        let outcome = file.save(&AlertState::default());
        assert!(matches!(outcome, Err(StateFileError::Write { .. })));
        assert_eq!(
            fs::read(dir.path().join("alerts.json")).expect("read"),
            before
        );
    }

    #[test]
    fn a_relative_path_without_a_directory_works() {
        let dir = TempDir::new().expect("tempdir");
        std::env::set_current_dir(dir.path()).expect("cwd");
        let file = StateFile::new("alerts.json".into());
        file.save(&state()).expect("save");
        assert_eq!(file.load().expect("load"), state());
    }
}
