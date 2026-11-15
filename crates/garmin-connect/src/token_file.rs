use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process;

use crate::{GarminError, Tokens};

const FILE_MODE: u32 = 0o600;
const DIR_MODE: u32 = 0o700;

/// The token file. Writers hold a [`TokenLock`] for their whole
/// read-refresh-write sequence.
#[derive(Debug, Clone)]
pub struct TokenFile {
    path: PathBuf,
}

/// An exclusive advisory lock on the token file's `.lock` sibling, released
/// when dropped.
#[derive(Debug)]
pub struct TokenLock {
    _file: File,
}

impl TokenFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Waits for, then takes, the exclusive lock shared by every process that
    /// writes this token file.
    pub fn lock(&self) -> Result<TokenLock, GarminError> {
        self.lock_file()
            .map(|file| TokenLock { _file: file })
            .map_err(|source| self.access_error(source))
    }

    pub fn load(&self) -> Result<Tokens, GarminError> {
        let bytes = fs::read(&self.path).map_err(|source| self.access_error(source))?;
        serde_json::from_slice(&bytes).map_err(|_| GarminError::InvalidTokenFile {
            path: self.path.clone(),
        })
    }

    /// Replaces the file atomically: a private temp file, fsynced, then renamed.
    pub fn save(&self, tokens: &Tokens) -> Result<(), GarminError> {
        self.write(tokens)
            .map_err(|source| self.access_error(source))
    }

    fn access_error(&self, source: io::Error) -> GarminError {
        GarminError::TokenFile {
            path: self.path.clone(),
            source,
        }
    }

    fn lock_path(&self) -> PathBuf {
        self.sibling("lock")
    }

    fn temp_path(&self) -> PathBuf {
        self.sibling(&format!("{}.tmp", process::id()))
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = OsString::from(self.path.as_os_str());
        name.push(".");
        name.push(suffix);
        PathBuf::from(name)
    }

    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    fn prepare_dir(&self) -> io::Result<()> {
        fs::create_dir_all(self.parent())?;
        fs::set_permissions(self.parent(), fs::Permissions::from_mode(DIR_MODE))
    }

    fn lock_file(&self) -> io::Result<File> {
        self.prepare_dir()?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(FILE_MODE)
            .open(self.lock_path())?;
        file.lock()?;
        Ok(file)
    }

    fn write(&self, tokens: &Tokens) -> io::Result<()> {
        self.prepare_dir()?;
        let json = serde_json::to_vec(tokens)?;
        let temp = self.temp_path();
        remove_if_present(&temp)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&temp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        fs::rename(&temp, &self.path)?;
        File::open(self.parent())?.sync_all()
    }
}

/// Removes this process's own leftover temp file; other writers' temp files
/// have other names and are never touched.
fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use crate::{GarminError, Tokens};

    use super::TokenFile;

    fn tokens(suffix: &str) -> Tokens {
        Tokens {
            di_token: format!("access-{suffix}"),
            di_refresh_token: format!("refresh-{suffix}"),
            di_client_id: "client".to_owned(),
        }
    }

    fn file_in(dir: &TempDir) -> TokenFile {
        TokenFile::new(dir.path().join("tokens").join("garmin_tokens.json"))
    }

    fn mode(path: &std::path::Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&tokens("a")).expect("save");
        assert_eq!(file.load().expect("load"), tokens("a"));
    }

    #[test]
    fn the_file_is_private_and_so_is_its_directory() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&tokens("a")).expect("save");
        assert_eq!(mode(&file.path), 0o600);
        assert_eq!(mode(file.path.parent().expect("parent")), 0o700);
    }

    #[test]
    fn two_saves_leave_one_valid_file_and_no_temp() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        file.save(&tokens("a")).expect("first save");
        file.save(&tokens("b")).expect("second save");
        assert_eq!(file.load().expect("load"), tokens("b"));
        assert!(!file.temp_path().exists());
    }

    #[test]
    fn a_leftover_temp_of_this_process_is_replaced() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        fs::create_dir_all(file.path.parent().expect("parent")).expect("dir");
        fs::write(file.temp_path(), "stale").expect("stale temp");
        file.save(&tokens("a")).expect("save");
        assert_eq!(file.load().expect("load"), tokens("a"));
    }

    #[test]
    fn another_writers_temp_file_is_left_alone() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        fs::create_dir_all(file.path.parent().expect("parent")).expect("dir");
        let other = file.sibling("1.tmp");
        fs::write(&other, "theirs").expect("other temp");
        file.save(&tokens("a")).expect("save");
        assert_eq!(fs::read_to_string(other).expect("still there"), "theirs");
    }

    #[test]
    fn temp_names_are_per_process() {
        let file = TokenFile::new("/tokens/garmin_tokens.json".into());
        let name = file.temp_path();
        assert_eq!(
            name.to_string_lossy(),
            format!("/tokens/garmin_tokens.json.{}.tmp", std::process::id())
        );
    }

    #[test]
    fn a_relative_path_saves_in_the_current_directory_parent() {
        let file = TokenFile::new("garmin_tokens.json".into());
        assert_eq!(file.parent(), std::path::Path::new("."));
    }

    #[test]
    fn the_lock_file_is_created_and_the_lock_is_exclusive() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        let guard = file.lock().expect("lock");
        assert!(file.lock_path().exists());
        let contender = fs::File::open(file.lock_path()).expect("open lock");
        assert!(contender.try_lock().is_err(), "held lock excludes others");
        drop(guard);
        assert!(contender.try_lock().is_ok(), "released lock is free");
    }

    #[test]
    fn a_missing_file_is_a_token_file_error() {
        let dir = TempDir::new().expect("tempdir");
        let error = file_in(&dir).load().expect_err("missing");
        assert!(matches!(error, GarminError::TokenFile { .. }));
    }

    #[test]
    fn garbage_is_invalid_without_echoing_the_content() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        fs::create_dir_all(file.path.parent().expect("parent")).expect("dir");
        fs::write(&file.path, "{\"di_token\": 12, secret-material").expect("write");
        let error = file.load().expect_err("invalid");
        assert!(matches!(error, GarminError::InvalidTokenFile { .. }));
        assert!(!format!("{error:?}{error}").contains("secret-material"));
    }

    #[test]
    fn a_python_garminconnect_file_loads() {
        let dir = TempDir::new().expect("tempdir");
        let file = file_in(&dir);
        fs::create_dir_all(file.path.parent().expect("parent")).expect("dir");
        let sample =
            r#"{"di_token": "a", "di_refresh_token": "r", "di_client_id": "c", "extra": 1}"#;
        fs::write(&file.path, sample).expect("write");
        let loaded = file.load().expect("load");
        assert_eq!(loaded.di_refresh_token, "r");
    }

    #[test]
    fn debug_hides_the_tokens() {
        let rendered = format!("{:?}", tokens("hunter2"));
        assert!(!rendered.contains("hunter2"));
    }
}
