use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Replaces `path` with `bytes` through a synced temp file, so a crash leaves
/// the old or the new content, never a mix.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let mut file = File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    File::open(parent_dir(path))?.sync_all()
}

/// The directory holding `path`; `.` for a bare file name.
fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn a_bare_file_name_is_synced_through_the_current_directory() {
        assert_eq!(super::parent_dir(Path::new("alerts.json")), Path::new("."));
        assert_eq!(
            super::parent_dir(Path::new("/data/alerts.json")),
            Path::new("/data")
        );
    }
}
