use std::fs::File;
use std::io;
use std::path::Path;

/// Makes the entries created or removed in `dir` durable.
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}
