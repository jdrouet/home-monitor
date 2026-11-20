use chrono::NaiveDate;
use domain::{ImportCounts, ImportFailure, Timestamp};
use tracing::{error, info};

use crate::archive_documents::{Kept, keep_missing};
use crate::import_window::{open_session, turn};
use crate::{
    ArchiveKey, ArchiveSource, BackfillCursor, BackfillStore, DocumentKey, RawArchive, RunEnd,
    RunOutcome,
};

/// Keeps Garmin's original documents of completed days, resuming from a
/// persisted rolling cursor, and stops before `until` so the daily pull is not
/// delayed.
pub struct ArchiveDays<'a, H, A, S> {
    pub source: &'a H,
    pub archive: &'a A,
    pub store: &'a S,
}

impl<H: ArchiveSource, A: RawArchive, S: BackfillStore> ArchiveDays<'_, H, A, S> {
    /// Archives days until the range is done, `until` is near, or a step
    /// fails. `plan` turns the stored cursor, if any, into the range to work
    /// on. A day is passed only once every one of its documents is archived.
    pub async fn run(
        &self,
        plan: impl FnOnce(Option<BackfillCursor>) -> BackfillCursor,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
    ) -> RunOutcome {
        let mut counts = ImportCounts::default();
        let end = self.steps(plan, until, clock, &mut counts).await;
        RunOutcome { end, counts }
    }

    async fn steps(
        &self,
        plan: impl FnOnce(Option<BackfillCursor>) -> BackfillCursor,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
        counts: &mut ImportCounts,
    ) -> RunEnd {
        let mut cursor = match self.cursor(plan) {
            Ok(cursor) => cursor,
            Err(end) => return end,
        };
        let mut opened = None;
        while !cursor.is_done() {
            let open = match open_session(self.source, opened.take(), until, &clock).await {
                Ok(open) => open,
                Err(end) => return end,
            };
            if !turn(until, &clock).await {
                return RunEnd::OutOfTime;
            }
            let date = cursor.next;
            match keep_missing(
                self.source,
                self.archive,
                &open,
                &items::<H>(date),
                until,
                &clock,
            )
            .await
            {
                Kept::Complete { fetched } => info!(%date, fetched, "day archived"),
                Kept::Deadline => return RunEnd::OutOfTime,
                Kept::SessionAge => continue,
                Kept::Stopped(reason) => return RunEnd::Stopped(reason),
            }
            let next = cursor.advanced();
            if let Err(error) = self.store.save(&next) {
                error!(%error, "cannot save the archive cursor, stopping");
                return RunEnd::Stopped(ImportFailure::ProgressNotSaved);
            }
            counts.stored += 1;
            cursor = next;
            opened = Some(open);
        }
        info!("day archive is up to date");
        RunEnd::Done
    }

    /// The cursor to work from, or why there is nothing to do.
    fn cursor(
        &self,
        plan: impl FnOnce(Option<BackfillCursor>) -> BackfillCursor,
    ) -> Result<BackfillCursor, RunEnd> {
        let stored = self.store.load().map_err(|error| {
            error!(%error, "cannot read the archive cursor, skipping the archive");
            RunEnd::Skipped
        })?;
        let planned = plan(stored);
        if Some(planned) != stored {
            self.store.save(&planned).map_err(|error| {
                error!(%error, "cannot save the archive cursor, skipping the archive");
                RunEnd::Stopped(ImportFailure::ProgressNotSaved)
            })?;
        }
        if planned.is_done() {
            return Err(RunEnd::Done);
        }
        Ok(planned)
    }
}

/// Every daily document of `date`, with its archive key.
fn items<H: ArchiveSource>(date: NaiveDate) -> Vec<(H::Document, DocumentKey, ArchiveKey)> {
    H::DAILY
        .iter()
        .map(|document| {
            let file = H::file_name(*document);
            (
                *document,
                DocumentKey::Day(date),
                ArchiveKey::Day { date, file },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::NaiveDate;
    use domain::{ImportCounts, ImportFailure};

    use crate::test_archive::{
        MemoryArchive, MemoryBackfill, ScriptedSource, Stored, TestError, after, clock,
    };
    use crate::{ArchiveDays, ArchiveKey, BackfillCursor, Fetched, RunEnd, RunOutcome};

    const DAY: Duration = Duration::from_secs(86_400);

    fn jan(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, day).expect("date")
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 15).expect("date")
    }

    fn key(date: NaiveDate, file: &'static str) -> ArchiveKey {
        ArchiveKey::Day { date, file }
    }

    fn days(first: u32, last: u32) -> BackfillCursor {
        BackfillCursor {
            next: jan(first),
            last: jan(last),
        }
    }

    async fn run(
        source: &ScriptedSource,
        archive: &MemoryArchive,
        store: &MemoryBackfill,
        window: Duration,
    ) -> RunEnd {
        outcome(source, archive, store, window).await.end
    }

    async fn outcome(
        source: &ScriptedSource,
        archive: &MemoryArchive,
        store: &MemoryBackfill,
        window: Duration,
    ) -> RunOutcome {
        ArchiveDays {
            source,
            archive,
            store,
        }
        .run(
            |stored| stored.unwrap_or_else(|| BackfillCursor::rolling(None, today(), &[])),
            after(window),
            clock(),
        )
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_plan_is_saved_before_anything_is_fetched() {
        let source = ScriptedSource {
            open_fails: true,
            ..ScriptedSource::default()
        };
        let store = MemoryBackfill::default();
        let end = run(&source, &MemoryArchive::default(), &store, DAY).await;
        assert_eq!(end, RunEnd::Stopped(ImportFailure::BadAnswer));
        assert_eq!(store.saves(), [BackfillCursor::rolling(None, today(), &[])]);
        assert!(source.requested().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn every_document_of_every_day_is_archived_and_the_range_is_done() {
        let (source, archive) = (ScriptedSource::default(), MemoryArchive::default());
        let store = MemoryBackfill::holding(days(1, 2));
        assert_eq!(run(&source, &archive, &store, DAY).await, RunEnd::Done);
        assert_eq!(archive.count(), 4);
        assert_eq!(
            archive.stored(&key(jan(2), "b.json")),
            Some(Stored::Document(b"b.json".to_vec()))
        );
        assert!(store.stored().expect("cursor").is_done());
        assert_eq!(source.opens(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn present_documents_are_not_fetched_again_and_an_empty_one_counts() {
        let (source, archive) = (ScriptedSource::default(), MemoryArchive::default());
        archive.entries.lock().expect("lock").insert(
            format!("{:?}", key(jan(1), "a.json")),
            Stored::Document(Vec::new()),
        );
        let store = MemoryBackfill::holding(days(1, 1));
        run(&source, &archive, &store, DAY).await;
        let requested = source.requested();
        assert_eq!(requested.len(), 1);
        assert_eq!(requested[0].0, "b.json");
    }

    #[tokio::test(start_paused = true)]
    async fn a_complete_day_costs_no_request() {
        let (source, archive) = (ScriptedSource::default(), MemoryArchive::default());
        let store = MemoryBackfill::holding(days(1, 1));
        run(&source, &archive, &store, DAY).await;
        let again = MemoryBackfill::holding(days(1, 1));
        run(&source, &archive, &again, DAY).await;
        assert_eq!(source.requested().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn no_data_and_denied_documents_are_recorded_and_the_day_advances() {
        let (source, archive) = (ScriptedSource::default(), MemoryArchive::default());
        source.script([Ok(Fetched::Denied), Ok(Fetched::Absent)]);
        let store = MemoryBackfill::holding(days(1, 1));
        assert_eq!(run(&source, &archive, &store, DAY).await, RunEnd::Done);
        assert_eq!(archive.stored(&key(jan(1), "a.json")), Some(Stored::Denied));
        assert_eq!(
            archive.stored(&key(jan(1), "b.json")),
            Some(Stored::Document(Vec::new()))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_document_stops_without_advancing_and_only_the_missing_is_redone() {
        for (error, reason) in [
            (TestError::Session, ImportFailure::LoginRejected),
            (TestError::Throttled, ImportFailure::Throttled),
            (TestError::Other, ImportFailure::BadAnswer),
        ] {
            let (source, archive) = (ScriptedSource::default(), MemoryArchive::default());
            source.script([Ok(Fetched::Body(vec![1])), Err(error)]);
            let store = MemoryBackfill::holding(days(1, 2));
            assert_eq!(
                run(&source, &archive, &store, DAY).await,
                RunEnd::Stopped(reason)
            );
            assert_eq!(store.stored(), Some(days(1, 2)));
            assert_eq!(archive.count(), 1);
            assert_eq!(run(&source, &archive, &store, DAY).await, RunEnd::Done);
            let files: Vec<_> = source.requested().iter().map(|(file, _)| *file).collect();
            assert_eq!(files, ["a.json", "b.json", "b.json", "a.json", "b.json"]);
            assert_eq!(archive.count(), 4);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_unavailable_or_unwritable_archive_stops_the_run() {
        let unavailable = MemoryArchive {
            contains_fails: true,
            ..MemoryArchive::default()
        };
        let unwritable = MemoryArchive {
            put_fails: true,
            ..MemoryArchive::default()
        };
        for (archive, fetches) in [(unavailable, 0), (unwritable, 1)] {
            let source = ScriptedSource::default();
            let store = MemoryBackfill::holding(days(1, 2));
            assert_eq!(
                run(&source, &archive, &store, DAY).await,
                RunEnd::Stopped(ImportFailure::ArchiveUnavailable)
            );
            assert_eq!(source.requested().len(), fetches);
            assert_eq!(store.stored(), Some(days(1, 2)));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_deadline_between_two_documents_ends_the_run_without_advancing() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(20),
            ..ScriptedSource::default()
        };
        let archive = MemoryArchive::default();
        let store = MemoryBackfill::holding(days(1, 1));
        let end = run(&source, &archive, &store, Duration::from_secs(430)).await;
        assert_eq!(end, RunEnd::OutOfTime);
        assert_eq!(store.stored(), Some(days(1, 1)));
        assert_eq!(archive.count(), 1);
        assert_eq!(run(&source, &archive, &store, DAY).await, RunEnd::Done);
        assert_eq!(source.requested().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_aging_session_is_replaced_between_documents() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(1000),
            ..ScriptedSource::default()
        };
        let archive = MemoryArchive::default();
        let store = MemoryBackfill::holding(days(1, 2));
        assert_eq!(run(&source, &archive, &store, DAY).await, RunEnd::Done);
        assert_eq!(source.opens(), 2);
        assert_eq!(archive.count(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_cursor_skips_the_archive() {
        let source = ScriptedSource::default();
        let store = MemoryBackfill {
            load_fails: true,
            ..MemoryBackfill::default()
        };
        let end = run(&source, &MemoryArchive::default(), &store, DAY).await;
        assert_eq!(end, RunEnd::Skipped);
        assert_eq!(source.opens(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_denied_marker_write_stops_with_the_archive_reason() {
        let source = ScriptedSource::default();
        source.script([Ok(Fetched::Denied)]);
        let archive = MemoryArchive {
            put_fails: true,
            ..MemoryArchive::default()
        };
        let store = MemoryBackfill::holding(days(1, 1));
        let end = run(&source, &archive, &store, DAY).await;
        assert_eq!(end, RunEnd::Stopped(ImportFailure::ArchiveUnavailable));
        assert_eq!(store.stored(), Some(days(1, 1)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_save_stops_with_its_reason_and_the_cursor_stays() {
        let first = MemoryBackfill {
            save_fails: true,
            ..MemoryBackfill::default()
        };
        let later = MemoryBackfill {
            save_fails: true,
            stored: std::sync::Mutex::new(Some(days(1, 2))),
            ..MemoryBackfill::default()
        };
        for (store, requests) in [(&first, 0), (&later, 2)] {
            let source = ScriptedSource::default();
            let end = run(&source, &MemoryArchive::default(), store, DAY).await;
            assert_eq!(end, RunEnd::Stopped(ImportFailure::ProgressNotSaved));
            assert_eq!(source.requested().len(), requests);
        }
        assert_eq!(first.stored(), None);
        assert_eq!(later.stored(), Some(days(1, 2)));
    }

    #[tokio::test(start_paused = true)]
    async fn the_stored_count_is_the_days_passed_including_after_a_session_reopen() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(1000),
            ..ScriptedSource::default()
        };
        let store = MemoryBackfill::holding(days(1, 2));
        let done = outcome(&source, &MemoryArchive::default(), &store, DAY).await;
        assert_eq!(done.end, RunEnd::Done);
        assert_eq!(
            done.counts,
            ImportCounts {
                stored: 2,
                ..ImportCounts::default()
            }
        );
        assert_eq!(source.opens(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_deadline_is_a_pause_that_keeps_the_days_passed() {
        let source = ScriptedSource {
            document_delay: Duration::from_secs(20),
            ..ScriptedSource::default()
        };
        let store = MemoryBackfill::holding(days(1, 3));
        let window = Duration::from_secs(520);
        let paused = outcome(&source, &MemoryArchive::default(), &store, window).await;
        assert_eq!(paused.end, RunEnd::OutOfTime);
        assert_eq!(paused.counts.stored, 1);
    }
}
