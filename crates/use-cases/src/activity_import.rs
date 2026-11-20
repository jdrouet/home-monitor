use domain::{ImportCounts, ImportFailure, Timestamp};
use tracing::{error, info, warn};

use crate::activity_cursor::ACTIVITY_PAGE;
use crate::archive_documents::{Kept, keep_missing};
use crate::import_window::{Fit, Opened, fits, open_session, turn};
use crate::{
    ActivityCursor, ActivityCursorStore, ArchiveKey, ArchiveSource, DocumentKey, RawArchive,
    RunEnd, RunOutcome,
};

/// Pages a catch-up may read in one run; the cursor remembers where it got.
const CATCH_UP_PAGES: u32 = 10;

/// Keeps Garmin's original documents of every activity: the past ones page by
/// page, newest first, then the new ones after each completed pull. It stops
/// before `until` so the daily pull is not delayed.
pub struct ActivityImport<'a, H, A, C> {
    pub source: &'a H,
    pub archive: &'a A,
    pub store: &'a C,
}

/// A page whose documents are all archived.
struct Done {
    len: u32,
    /// The page holds the cursor's boundary activity.
    reached: bool,
}

/// Why a step did not complete.
enum Stop {
    /// The session is due for replacement; the step is to be redone.
    Retry,
    End(RunEnd),
}

impl<H: ArchiveSource, A: RawArchive, C: ActivityCursorStore> ActivityImport<'_, H, A, C> {
    /// Walks the history while it is not done, else catches up with new
    /// activities. The cursor moves only past a page whose documents are all
    /// archived, and from the offset that page was read at.
    pub async fn run(&self, until: Timestamp, clock: impl Fn() -> Timestamp) -> RunOutcome {
        let mut counts = ImportCounts::default();
        let end = self.steps(until, &clock, &mut counts).await;
        RunOutcome { end, counts }
    }

    async fn steps(
        &self,
        until: Timestamp,
        clock: &impl Fn() -> Timestamp,
        counts: &mut ImportCounts,
    ) -> RunEnd {
        let cursor = match self.cursor() {
            Ok(cursor) => cursor,
            Err(end) => return end,
        };
        if cursor.done {
            self.catch_up(cursor, until, clock, counts).await
        } else {
            self.history(cursor, until, clock, counts).await
        }
    }

    async fn history(
        &self,
        mut cursor: ActivityCursor,
        until: Timestamp,
        clock: &impl Fn() -> Timestamp,
        counts: &mut ImportCounts,
    ) -> RunEnd {
        let mut offset = cursor.resume_start();
        let mut opened = None;
        while !cursor.done {
            let open = match open_session(self.source, opened.take(), until, clock).await {
                Ok(open) => open,
                Err(end) => return end,
            };
            if !turn(until, clock).await {
                return RunEnd::OutOfTime;
            }
            cursor = match self.begin(&open, cursor, until, clock).await {
                Ok(begun) => begun,
                Err(Stop::Retry) => continue,
                Err(Stop::End(end)) => return end,
            };
            let len = match self
                .page(&open, offset, cursor.boundary, until, clock, counts)
                .await
            {
                Ok(done) => done.len,
                Err(Stop::Retry) => continue,
                Err(Stop::End(end)) => return end,
            };
            let next = cursor.after_history_page(offset, len);
            if !self.save(&next) {
                return RunEnd::Stopped(ImportFailure::ProgressNotSaved);
            }
            offset = next.start;
            cursor = next;
            opened = Some(open);
        }
        info!("activity history archived");
        RunEnd::Done
    }

    async fn catch_up(
        &self,
        mut cursor: ActivityCursor,
        until: Timestamp,
        clock: &impl Fn() -> Timestamp,
        counts: &mut ImportCounts,
    ) -> RunEnd {
        let mut offset = cursor.catch_up_start();
        let mut opened = None;
        let mut pages = 0;
        while pages < CATCH_UP_PAGES {
            let open = match open_session(self.source, opened.take(), until, clock).await {
                Ok(open) => open,
                Err(end) => return end,
            };
            if !turn(until, clock).await {
                return RunEnd::OutOfTime;
            }
            cursor = match self.begin(&open, cursor, until, clock).await {
                Ok(begun) => begun,
                Err(Stop::Retry) => continue,
                Err(Stop::End(end)) => return end,
            };
            let done = match self
                .page(&open, offset, cursor.boundary, until, clock, counts)
                .await
            {
                Ok(done) => done,
                Err(Stop::Retry) => continue,
                Err(Stop::End(end)) => return end,
            };
            let next = cursor.after_catch_up_page(offset, done.len, done.reached);
            if next != cursor && !self.save(&next) {
                return RunEnd::Stopped(ImportFailure::ProgressNotSaved);
            }
            cursor = next;
            if cursor.catch_up.is_none() {
                return RunEnd::Done;
            }
            offset += done.len;
            opened = Some(open);
            pages += 1;
        }
        RunEnd::Capped
    }

    /// Notes the newest activity as the start of this pass and saves it before
    /// anything is archived. An empty list has nothing to note.
    async fn begin(
        &self,
        open: &Opened<H::Session>,
        cursor: ActivityCursor,
        until: Timestamp,
        clock: &impl Fn() -> Timestamp,
    ) -> Result<ActivityCursor, Stop> {
        if cursor.pending.is_some() {
            return Ok(cursor);
        }
        check(open, until, clock)?;
        let newest = match self.source.activities(&open.session, 0, 1).await {
            Ok(refs) => refs.first().map(|activity| activity.id.0),
            Err(error) => {
                warn!(%error, "cannot list activities, stopping");
                return Err(Stop::End(RunEnd::Stopped(H::failure(&error))));
            }
        };
        let Some(id) = newest else {
            return Ok(cursor);
        };
        let begun = cursor.with_pending(id);
        if self.save(&begun) {
            Ok(begun)
        } else {
            Err(Stop::End(RunEnd::Stopped(ImportFailure::ProgressNotSaved)))
        }
    }

    /// Reads the page of the list at `offset` and archives what is missing of
    /// its activities, one activity at a time. `counts` gains each activity
    /// that needed a new document.
    async fn page(
        &self,
        open: &Opened<H::Session>,
        offset: u32,
        boundary: Option<u64>,
        until: Timestamp,
        clock: &impl Fn() -> Timestamp,
        counts: &mut ImportCounts,
    ) -> Result<Done, Stop> {
        check(open, until, clock)?;
        let refs = match self
            .source
            .activities(&open.session, offset, ACTIVITY_PAGE)
            .await
        {
            Ok(refs) => refs,
            Err(error) => {
                warn!(%error, offset, "cannot list activities, stopping");
                return Err(Stop::End(RunEnd::Stopped(H::failure(&error))));
            }
        };
        for activity in &refs {
            let items: Vec<_> = H::ACTIVITY
                .iter()
                .map(|document| {
                    let key = ArchiveKey::Activity {
                        activity: *activity,
                        file: H::file_name(*document),
                    };
                    (*document, DocumentKey::Activity(activity.id), key)
                })
                .collect();
            match keep_missing(self.source, self.archive, open, &items, until, clock).await {
                Kept::Complete { fetched } => counts.stored += u32::from(fetched > 0),
                Kept::Deadline => return Err(Stop::End(RunEnd::OutOfTime)),
                Kept::SessionAge => return Err(Stop::Retry),
                Kept::Stopped(reason) => return Err(Stop::End(RunEnd::Stopped(reason))),
            }
        }
        Ok(Done {
            len: u32::try_from(refs.len()).unwrap_or(ACTIVITY_PAGE),
            reached: boundary.is_some_and(|id| refs.iter().any(|activity| activity.id.0 == id)),
        })
    }

    /// The stored cursor, a fresh one saved first when there is none.
    fn cursor(&self) -> Result<ActivityCursor, RunEnd> {
        let stored = self.store.load().map_err(|error| {
            error!(%error, "cannot read the activity cursor, skipping the activities");
            RunEnd::Skipped
        })?;
        if let Some(stored) = stored {
            return Ok(stored);
        }
        let fresh = ActivityCursor::fresh();
        self.store.save(&fresh).map_err(|error| {
            error!(%error, "cannot save the activity cursor, skipping the activities");
            RunEnd::Stopped(ImportFailure::ProgressNotSaved)
        })?;
        Ok(fresh)
    }

    fn save(&self, cursor: &ActivityCursor) -> bool {
        match self.store.save(cursor) {
            Ok(()) => true,
            Err(error) => {
                error!(%error, "cannot save the activity cursor, stopping");
                false
            }
        }
    }
}

/// Whether a request may start now in the session.
fn check<T>(
    open: &Opened<T>,
    until: Timestamp,
    clock: &impl Fn() -> Timestamp,
) -> Result<(), Stop> {
    match fits(until, open.at, clock) {
        Fit::Yes => Ok(()),
        Fit::Deadline => Err(Stop::End(RunEnd::OutOfTime)),
        Fit::SessionAge => Err(Stop::Retry),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use chrono::NaiveDate;
    use domain::ImportFailure;
    use tokio::time::Instant;

    use crate::test_archive::{
        MemoryArchive, MemoryCursor, ScriptedSource, TestError, after, clock, stored,
    };
    use crate::{
        ActivityCursor, ActivityId, ActivityImport, ActivityRef, ArchiveKey, DocumentKey, Fetched,
        RunEnd, RunOutcome,
    };

    const DAY: Duration = Duration::from_secs(86_400);
    const FILES: [&str; 2] = ["x.json", "y.json"];

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).expect("date")
    }

    fn activity(id: u64) -> ActivityRef {
        ActivityRef {
            id: ActivityId(id),
            date: date(),
        }
    }

    fn key(id: u64, file: &'static str) -> ArchiveKey {
        ArchiveKey::Activity {
            activity: activity(id),
            file,
        }
    }

    fn source_with(ids: impl Iterator<Item = u64>) -> ScriptedSource {
        ScriptedSource {
            list: Mutex::new(ids.map(activity).collect()),
            ..ScriptedSource::default()
        }
    }

    fn seed(archive: &MemoryArchive, ids: impl Iterator<Item = u64>) {
        for id in ids {
            for file in FILES {
                archive.seed(&key(id, file));
            }
        }
    }

    fn has(archive: &MemoryArchive, id: u64) -> bool {
        FILES
            .iter()
            .all(|file| archive.stored(&key(id, file)).is_some())
    }

    fn history(start: u32) -> ActivityCursor {
        ActivityCursor {
            start,
            ..ActivityCursor::fresh()
        }
    }

    fn caught_up(catch_up: Option<u32>) -> ActivityCursor {
        ActivityCursor {
            start: 500,
            done: true,
            catch_up,
            ..ActivityCursor::fresh()
        }
    }

    fn caught_up_to(boundary: u64) -> ActivityCursor {
        ActivityCursor {
            boundary: Some(boundary),
            ..caught_up(None)
        }
    }

    async fn run_in(
        source: &ScriptedSource,
        archive: &MemoryArchive,
        store: &MemoryCursor,
        window: Duration,
    ) -> (RunOutcome, Duration) {
        let start = Instant::now();
        let outcome = ActivityImport {
            source,
            archive,
            store,
        }
        .run(after(window), clock())
        .await;
        (outcome, start.elapsed())
    }

    async fn outcome(
        source: &ScriptedSource,
        archive: &MemoryArchive,
        store: &MemoryCursor,
    ) -> RunOutcome {
        run_in(source, archive, store, DAY).await.0
    }

    async fn run(source: &ScriptedSource, archive: &MemoryArchive, store: &MemoryCursor) -> RunEnd {
        outcome(source, archive, store).await.end
    }

    #[tokio::test(start_paused = true)]
    async fn a_fresh_cursor_is_saved_before_the_first_page() {
        let source = ScriptedSource {
            open_fails: true,
            ..ScriptedSource::default()
        };
        let store = MemoryCursor::default();
        let end = run(&source, &MemoryArchive::default(), &store).await;
        assert_eq!(end, RunEnd::Stopped(ImportFailure::BadAnswer));
        assert_eq!(store.saves(), [ActivityCursor::fresh()]);
        assert!(source.pages().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_history_is_read_page_by_page_a_minute_apart_until_a_short_page() {
        let (source, archive) = (source_with(0..7), MemoryArchive::default());
        let store = MemoryCursor::default();
        let (outcome, elapsed) = run_in(&source, &archive, &store, DAY).await;
        assert_eq!(outcome.end, RunEnd::Done);
        assert_eq!(outcome.counts, stored(7));
        assert_eq!(source.pages(), [(0, 1), (0, 5), (5, 5)]);
        assert_eq!(elapsed, Duration::from_secs(180));
        assert!((0..7).all(|id| has(&archive, id)));
        assert_eq!(
            store.stored(),
            Some(ActivityCursor {
                start: 7,
                done: true,
                catch_up: None,
                boundary: Some(0),
                pending: None,
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_empty_list_finishes_the_history() {
        let (source, archive) = (source_with(0..0), MemoryArchive::default());
        let store = MemoryCursor::default();
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert_eq!(store.stored().map(|cursor| cursor.done), Some(true));
    }

    #[tokio::test(start_paused = true)]
    async fn the_history_resumes_a_page_early_and_checkpoints_from_the_page_read() {
        let (source, archive) = (source_with(0..105), MemoryArchive::default());
        seed(&archive, 0..100);
        let store = MemoryCursor::holding(history(100));
        let done = outcome(&source, &archive, &store).await;
        assert_eq!((done.end, done.counts), (RunEnd::Done, stored(5)));
        assert_eq!(source.pages(), [(0, 1), (95, 5), (100, 5), (105, 5)]);
        let starts: Vec<_> = store.saves().iter().map(|cursor| cursor.start).collect();
        assert_eq!(starts, [100, 100, 105, 105]);
        let fetched: Vec<_> = source.requested().iter().map(|(_, key)| *key).collect();
        assert!(
            fetched
                .iter()
                .all(|key| matches!(key, DocumentKey::Activity(ActivityId(id)) if *id >= 100))
        );
        assert_eq!(fetched.len(), 10);
    }

    #[tokio::test(start_paused = true)]
    async fn deleted_activities_between_runs_do_not_hide_unseen_ones() {
        let (source, archive) = (source_with(2..20), MemoryArchive::default());
        seed(&archive, 0..10);
        let store = MemoryCursor::holding(history(10));
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert!((2..20).all(|id| has(&archive, id)));
    }

    #[tokio::test(start_paused = true)]
    async fn the_catch_up_stops_on_the_page_holding_the_boundary() {
        let (source, archive) = (source_with(0..12), MemoryArchive::default());
        seed(&archive, 2..12);
        let store = MemoryCursor::holding(caught_up_to(7));
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert_eq!(source.pages(), [(0, 1), (0, 5), (5, 5)]);
        let saved: Vec<_> = store.saves().iter().map(|cursor| cursor.catch_up).collect();
        assert_eq!(saved, [None, Some(5), None]);
        assert!(has(&archive, 0) && has(&archive, 1));
        let end = store.stored().expect("cursor");
        assert_eq!((end.boundary, end.pending), (Some(0), None));
    }

    #[tokio::test(start_paused = true)]
    async fn new_activities_shifting_every_offset_do_not_end_a_resumed_catch_up_early() {
        let ids = (100..105).chain(0..12).chain(999..1004);
        let (source, archive) = (source_with(ids), MemoryArchive::default());
        seed(&archive, (0..5).chain(999..1004));
        let interrupted = ActivityCursor {
            catch_up: Some(5),
            boundary: Some(999),
            pending: Some(0),
            ..caught_up(None)
        };
        let store = MemoryCursor::holding(interrupted);
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert_eq!(source.pages(), [(0, 5), (5, 5), (10, 5), (15, 5)]);
        assert!((0..12).chain(100..105).all(|id| has(&archive, id)));
        let end = store.stored().expect("cursor");
        assert_eq!(
            (end.boundary, end.pending, end.catch_up),
            (Some(0), None, None)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_deleted_boundary_falls_back_to_walking_until_a_short_page() {
        let (source, archive) = (source_with(0..12), MemoryArchive::default());
        let store = MemoryCursor::holding(caught_up_to(9999));
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert_eq!(source.pages(), [(0, 1), (0, 5), (5, 5), (10, 5)]);
        assert!((0..12).all(|id| has(&archive, id)));
        assert_eq!(store.stored().and_then(|cursor| cursor.boundary), Some(0));
    }

    #[tokio::test(start_paused = true)]
    async fn a_caught_up_archive_reads_one_page_and_fetches_nothing() {
        let (source, archive) = (source_with(0..12), MemoryArchive::default());
        seed(&archive, 0..12);
        let store = MemoryCursor::holding(caught_up_to(3));
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert_eq!(source.pages(), [(0, 1), (0, 5)]);
        assert_eq!(store.saves().len(), 2);
        assert!(source.requested().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn an_interrupted_catch_up_resumes_with_an_overlap_and_finishes() {
        let (source, archive) = (source_with(0..8), MemoryArchive::default());
        seed(&archive, 0..5);
        let store = MemoryCursor::holding(caught_up(Some(5)));
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert_eq!(source.pages(), [(0, 1), (0, 5), (5, 5)]);
        assert!((0..8).all(|id| has(&archive, id)));
        assert_eq!(store.stored().map(|cursor| cursor.catch_up), Some(None));
    }

    #[tokio::test(start_paused = true)]
    async fn the_catch_up_cap_keeps_its_progress_for_the_next_run() {
        let (source, archive) = (source_with(0..100), MemoryArchive::default());
        let store = MemoryCursor::holding(caught_up(None));
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Capped);
        assert_eq!(source.pages().len(), 11);
        assert_eq!(
            store.stored(),
            Some(ActivityCursor {
                catch_up: Some(50),
                pending: Some(0),
                ..caught_up(None)
            })
        );
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Capped);
        assert_eq!(source.pages()[11], (45, 5));
        assert_eq!(store.stored().and_then(|cursor| cursor.catch_up), Some(95));
    }

    #[tokio::test(start_paused = true)]
    async fn failures_stop_without_moving_the_cursor() {
        let failing_list = ScriptedSource {
            list_fails: true,
            ..source_with(0..7)
        };
        let failing_document = source_with(0..7);
        failing_document.script([Err(TestError::Other)]);
        let unwritable = MemoryArchive {
            put_fails: true,
            ..MemoryArchive::default()
        };
        let unavailable = MemoryArchive {
            contains_fails: true,
            ..MemoryArchive::default()
        };
        let cases = [
            (
                &failing_list,
                MemoryArchive::default(),
                ImportFailure::BadAnswer,
            ),
            (
                &failing_document,
                MemoryArchive::default(),
                ImportFailure::BadAnswer,
            ),
            (
                &failing_document,
                unwritable,
                ImportFailure::ArchiveUnavailable,
            ),
            (
                &failing_document,
                unavailable,
                ImportFailure::ArchiveUnavailable,
            ),
        ];
        for (source, archive, reason) in &cases {
            let store = MemoryCursor::default();
            assert_eq!(run(source, archive, &store).await, RunEnd::Stopped(*reason));
            let stored = store.stored().expect("cursor");
            assert_eq!((stored.start, stored.done), (0, false));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_cursor_save_stops_the_run() {
        let (source, archive) = (source_with(0..7), MemoryArchive::default());
        let store = MemoryCursor::holding(history(0));
        let failing = MemoryCursor {
            save_fails: true,
            ..store
        };
        assert_eq!(
            run(&source, &archive, &failing).await,
            RunEnd::Stopped(ImportFailure::ProgressNotSaved)
        );
        assert_eq!(source.pages(), [(0, 1)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_page_checkpoint_keeps_the_cursor_and_counts_the_archived_workouts() {
        let (source, archive) = (source_with(0..7), MemoryArchive::default());
        let pending = ActivityCursor {
            pending: Some(0),
            ..history(0)
        };
        let store = MemoryCursor {
            save_fails: true,
            ..MemoryCursor::holding(pending)
        };
        let stopped = outcome(&source, &archive, &store).await;
        assert_eq!(
            stopped.end,
            RunEnd::Stopped(ImportFailure::ProgressNotSaved)
        );
        assert_eq!(stopped.counts, stored(5));
        assert_eq!(source.pages(), [(0, 5)]);
        assert_eq!(store.stored(), Some(pending));
        assert!((0..5).all(|id| has(&archive, id)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_first_save_stops_before_any_request() {
        let source = source_with(0..7);
        let store = MemoryCursor {
            save_fails: true,
            ..MemoryCursor::default()
        };
        let end = run(&source, &MemoryArchive::default(), &store).await;
        assert_eq!(end, RunEnd::Stopped(ImportFailure::ProgressNotSaved));
        assert_eq!(source.opens(), 0);
        assert_eq!(store.stored(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_page_list_stops_with_its_reason_and_the_cursor_stays() {
        let source = ScriptedSource {
            list_fails: true,
            ..source_with(0..7)
        };
        let pending = ActivityCursor {
            pending: Some(0),
            ..history(0)
        };
        let store = MemoryCursor::holding(pending);
        let end = run(&source, &MemoryArchive::default(), &store).await;
        assert_eq!(end, RunEnd::Stopped(ImportFailure::BadAnswer));
        assert_eq!(source.pages(), [(0, 5)]);
        assert_eq!(store.stored(), Some(pending));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_denied_marker_write_stops_with_the_archive_reason() {
        let source = source_with(0..1);
        source.script([Ok(Fetched::Denied)]);
        let archive = MemoryArchive {
            put_fails: true,
            ..MemoryArchive::default()
        };
        let store = MemoryCursor::default();
        let end = run(&source, &archive, &store).await;
        assert_eq!(end, RunEnd::Stopped(ImportFailure::ArchiveUnavailable));
        assert_eq!(source.requested().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_workout_completed_before_a_deadline_counts_and_costs_exactly_its_documents() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(20),
            ..source_with(0..3)
        };
        let (archive, store) = (MemoryArchive::default(), MemoryCursor::default());
        let (outcome, _) = run_in(&source, &archive, &store, Duration::from_secs(450)).await;
        assert_eq!(outcome.end, RunEnd::OutOfTime);
        assert_eq!(outcome.counts, stored(1));
        let zero = DocumentKey::Activity(ActivityId(0));
        assert_eq!(source.requested(), [("x.json", zero), ("y.json", zero)]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_deadline_inside_a_page_ends_the_run_without_moving_the_cursor() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(20),
            ..source_with(0..7)
        };
        let archive = MemoryArchive::default();
        let store = MemoryCursor::default();
        let (outcome, _) = run_in(&source, &archive, &store, Duration::from_secs(430)).await;
        assert_eq!(outcome.end, RunEnd::OutOfTime);
        let stored = store.stored().expect("cursor");
        assert_eq!((stored.start, stored.done), (0, false));
        assert_eq!(archive.count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_aging_session_is_replaced_and_the_page_redone() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(1000),
            ..source_with(0..2)
        };
        let (archive, store) = (MemoryArchive::default(), MemoryCursor::default());
        let done = outcome(&source, &archive, &store).await;
        assert_eq!((done.end, done.counts), (RunEnd::Done, stored(2)));
        assert_eq!(source.opens(), 2);
        assert!((0..2).all(|id| has(&archive, id)));
        assert_eq!(source.pages(), [(0, 1), (0, 5), (0, 5)]);
        let (zero, one) = (
            DocumentKey::Activity(ActivityId(0)),
            DocumentKey::Activity(ActivityId(1)),
        );
        assert_eq!(
            source.requested(),
            [
                ("x.json", zero),
                ("y.json", zero),
                ("x.json", one),
                ("y.json", one)
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_cursor_skips_the_activities() {
        let source = ScriptedSource::default();
        let store = MemoryCursor {
            load_fails: true,
            ..MemoryCursor::default()
        };
        let end = run(&source, &MemoryArchive::default(), &store).await;
        assert_eq!(end, RunEnd::Skipped);
        assert_eq!(source.opens(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn no_data_and_denied_documents_complete_the_page() {
        let (source, archive) = (source_with(0..1), MemoryArchive::default());
        source.script([Ok(Fetched::Denied), Ok(Fetched::Absent)]);
        let store = MemoryCursor::default();
        assert_eq!(run(&source, &archive, &store).await, RunEnd::Done);
        assert!(has(&archive, 0));
    }
}
