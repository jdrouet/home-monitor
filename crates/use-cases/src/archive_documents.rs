use domain::Timestamp;
use tracing::warn;

use crate::import_window::{Fit, Opened, fits};
use crate::{ArchiveKey, ArchiveSource, DocumentKey, Fetched, RawArchive};

/// How a pass over a set of documents ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kept {
    /// Every document is in the archive, `fetched` of them new.
    Complete { fetched: usize },
    /// The next request would cross the margin before the deadline.
    Deadline,
    /// The session is due for replacement.
    SessionAge,
    /// Garmin or the archive failed; the run resumes after the next pull.
    Stopped,
}

/// Fetches and stores the documents the archive does not hold yet, in order.
/// Garmin's 404 and 204 are kept as empty documents and a 403 as a denied
/// marker, so none of them is asked for twice. The first failure stops the
/// pass; bodies and URLs never reach the log.
pub(crate) async fn keep_missing<H: ArchiveSource, A: RawArchive>(
    source: &H,
    archive: &A,
    opened: &Opened<H::Session>,
    items: &[(H::Document, DocumentKey, ArchiveKey)],
    until: Timestamp,
    clock: &impl Fn() -> Timestamp,
) -> Kept {
    let mut fetched = 0;
    for (document, key, archive_key) in items {
        let file = H::file_name(*document);
        match archive.contains(archive_key) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                warn!(%error, file, "cannot look in the archive, stopping");
                return Kept::Stopped;
            }
        }
        match fits(until, opened.at, clock) {
            Fit::Yes => {}
            Fit::Deadline => return Kept::Deadline,
            Fit::SessionAge => return Kept::SessionAge,
        }
        if !fetch(source, archive, opened, *document, *key, archive_key).await {
            return Kept::Stopped;
        }
        fetched += 1;
    }
    Kept::Complete { fetched }
}

/// Fetches one document and stores the outcome; `false` when either failed.
async fn fetch<H: ArchiveSource, A: RawArchive>(
    source: &H,
    archive: &A,
    opened: &Opened<H::Session>,
    document: H::Document,
    key: DocumentKey,
    archive_key: &ArchiveKey,
) -> bool {
    let file = H::file_name(document);
    let fetched = match source.document(&opened.session, document, key).await {
        Ok(fetched) => fetched,
        Err(error) => {
            log_failure::<H>(&error, file);
            return false;
        }
    };
    let stored = match fetched {
        Fetched::Body(bytes) => archive.put(archive_key, &bytes),
        Fetched::Absent => archive.put(archive_key, &[]),
        Fetched::Denied => archive.put_denied(archive_key),
    };
    if let Err(error) = stored {
        warn!(%error, file, "cannot write to the archive, stopping");
        return false;
    }
    true
}

fn log_failure<H: ArchiveSource>(error: &H::Error, file: &str) {
    if H::is_session_error(error) {
        warn!(%error, file, "Garmin refused the session, stopping");
    } else if H::is_throttled(error) {
        warn!(%error, file, "Garmin asks to slow down, stopping");
    } else {
        warn!(%error, file, "cannot fetch a document, stopping");
    }
}
