//! In-memory stand-ins for the archive ports, shared by the runner tests.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::sync::Mutex;
use std::time::Duration;

use chrono::NaiveDate;
use tokio::time::sleep;

use crate::{
    ACTIVITY_PAGE, ActivityCursor, ActivityCursorStore, ActivityRef, ArchiveKey, ArchiveSource,
    DailyHealth, DocumentKey, Fetched, HealthProvider, RawArchive,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestError {
    Session,
    Throttled,
    Other,
}

impl fmt::Display for TestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("test error")
    }
}

impl std::error::Error for TestError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TestDocument(pub &'static str);

/// A source that answers every document with its own file name unless a
/// script says otherwise, and serves `list` as the activity list.
#[derive(Default)]
pub(crate) struct ScriptedSource {
    pub open_fails: bool,
    pub list_fails: bool,
    pub document_delay: Duration,
    pub opens: Mutex<usize>,
    pub requested: Mutex<Vec<(&'static str, DocumentKey)>>,
    pub script: Mutex<VecDeque<Result<Fetched, TestError>>>,
    pub list: Mutex<Vec<ActivityRef>>,
    pub pages: Mutex<Vec<(u32, u32)>>,
}

impl ScriptedSource {
    pub(crate) fn opens(&self) -> usize {
        *self.opens.lock().expect("lock")
    }

    pub(crate) fn requested(&self) -> Vec<(&'static str, DocumentKey)> {
        self.requested.lock().expect("lock").clone()
    }

    pub(crate) fn pages(&self) -> Vec<(u32, u32)> {
        self.pages.lock().expect("lock").clone()
    }

    pub(crate) fn script(&self, outcomes: impl IntoIterator<Item = Result<Fetched, TestError>>) {
        self.script.lock().expect("lock").extend(outcomes);
    }
}

impl HealthProvider for ScriptedSource {
    type Session = ();
    type Error = TestError;

    async fn open(&self) -> Result<(), TestError> {
        *self.opens.lock().expect("lock") += 1;
        if self.open_fails {
            Err(TestError::Other)
        } else {
            Ok(())
        }
    }

    async fn daily(&self, _session: &(), _date: NaiveDate) -> Result<DailyHealth, TestError> {
        Ok(DailyHealth::default())
    }

    fn is_session_error(error: &TestError) -> bool {
        *error == TestError::Session
    }

    fn is_throttled(error: &TestError) -> bool {
        *error == TestError::Throttled
    }
}

impl ArchiveSource for ScriptedSource {
    type Document = TestDocument;

    const DAILY: &'static [TestDocument] = &[TestDocument("a.json"), TestDocument("b.json")];
    const ACTIVITY: &'static [TestDocument] = &[TestDocument("x.json"), TestDocument("y.json")];

    fn file_name(document: TestDocument) -> &'static str {
        document.0
    }

    async fn document(
        &self,
        _session: &(),
        document: TestDocument,
        key: DocumentKey,
    ) -> Result<Fetched, TestError> {
        self.requested.lock().expect("lock").push((document.0, key));
        sleep(self.document_delay).await;
        let scripted = self.script.lock().expect("lock").pop_front();
        scripted.unwrap_or_else(|| Ok(Fetched::Body(document.0.as_bytes().to_vec())))
    }

    async fn activities(
        &self,
        _session: &(),
        start: u32,
        limit: u32,
    ) -> Result<Vec<ActivityRef>, TestError> {
        assert_eq!(limit, ACTIVITY_PAGE);
        self.pages.lock().expect("lock").push((start, limit));
        if self.list_fails {
            return Err(TestError::Other);
        }
        let list = self.list.lock().expect("lock");
        let from = usize::try_from(start).expect("fits").min(list.len());
        let to = usize::try_from(start + limit)
            .expect("fits")
            .min(list.len());
        Ok(list[from..to].to_vec())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stored {
    Document(Vec<u8>),
    Denied,
}

/// An archive in memory; the switches make its calls fail.
#[derive(Default)]
pub(crate) struct MemoryArchive {
    pub contains_fails: bool,
    pub put_fails: bool,
    pub(crate) entries: Mutex<HashMap<String, Stored>>,
}

impl MemoryArchive {
    pub(crate) fn stored(&self, key: &ArchiveKey) -> Option<Stored> {
        self.entries
            .lock()
            .expect("lock")
            .get(&format!("{key:?}"))
            .cloned()
    }

    /// Marks `key` as stored, as an earlier run would have.
    pub(crate) fn seed(&self, key: &ArchiveKey) {
        self.insert(key, Stored::Document(Vec::new()))
            .expect("seeding does not fail");
    }

    pub(crate) fn count(&self) -> usize {
        self.entries.lock().expect("lock").len()
    }

    fn insert(&self, key: &ArchiveKey, stored: Stored) -> Result<(), io::Error> {
        if self.put_fails {
            return Err(io::Error::other("disk full"));
        }
        self.entries
            .lock()
            .expect("lock")
            .insert(format!("{key:?}"), stored);
        Ok(())
    }
}

impl RawArchive for MemoryArchive {
    type Error = io::Error;

    fn contains(&self, key: &ArchiveKey) -> Result<bool, io::Error> {
        if self.contains_fails {
            return Err(io::Error::other("not mounted"));
        }
        Ok(self.stored(key).is_some())
    }

    fn put(&self, key: &ArchiveKey, bytes: &[u8]) -> Result<(), io::Error> {
        self.insert(key, Stored::Document(bytes.to_vec()))
    }

    fn put_denied(&self, key: &ArchiveKey) -> Result<(), io::Error> {
        self.insert(key, Stored::Denied)
    }
}

/// An activity cursor in memory; the switches make its calls fail.
#[derive(Default)]
pub(crate) struct MemoryCursor {
    pub load_fails: bool,
    pub save_fails: bool,
    pub stored: Mutex<Option<ActivityCursor>>,
    pub saves: Mutex<Vec<ActivityCursor>>,
}

impl MemoryCursor {
    pub(crate) fn holding(cursor: ActivityCursor) -> Self {
        Self {
            stored: Mutex::new(Some(cursor)),
            ..Self::default()
        }
    }

    pub(crate) fn stored(&self) -> Option<ActivityCursor> {
        *self.stored.lock().expect("lock")
    }

    pub(crate) fn saves(&self) -> Vec<ActivityCursor> {
        self.saves.lock().expect("lock").clone()
    }
}

impl ActivityCursorStore for MemoryCursor {
    type Error = io::Error;

    fn load(&self) -> Result<Option<ActivityCursor>, io::Error> {
        if self.load_fails {
            return Err(io::Error::other("corrupt"));
        }
        Ok(self.stored())
    }

    fn save(&self, cursor: &ActivityCursor) -> Result<(), io::Error> {
        if self.save_fails {
            return Err(io::Error::other("disk full"));
        }
        self.saves.lock().expect("lock").push(*cursor);
        *self.stored.lock().expect("lock") = Some(*cursor);
        Ok(())
    }
}
