use chrono::NaiveDate;
use domain::{
    AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportFailure, Measurement, SourceId,
    SourceKind, Timestamp,
};
use tracing::{error, info, warn};

use crate::import_window::{SESSION_MAX_AGE, open_session, turn};
use crate::{BackfillCursor, BackfillStore, HealthProvider, Publisher};

/// Imports health days one at a time, resuming from a persisted cursor, and
/// stops before `until` so the daily pull is not delayed.
pub struct HealthBackfill<'a, H, P, S> {
    pub provider: &'a H,
    pub publisher: &'a P,
    pub store: &'a S,
    pub source_id: &'a SourceId,
}

/// How one day passed, or why the run must stop.
enum Step {
    Stored,
    Empty,
    /// The day failed for a reason confined to it.
    Skipped,
    Stop(ImportFailure),
}

/// Why a run ended. `Stopped` means Garmin or the disk refused something that
/// would refuse again at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// The cursor is done.
    Done,
    /// The next step does not fit before the deadline.
    OutOfTime,
    /// The cursor could not be read, so nothing was attempted.
    Skipped,
    /// A session, fetch, publish or save failed.
    Stopped(ImportFailure),
}

impl RunEnd {
    /// How this end reads as an attempt outcome.
    pub fn outcome(self) -> AttemptOutcome {
        match self {
            Self::Done => AttemptOutcome::Done,
            Self::OutOfTime => AttemptOutcome::Paused,
            Self::Skipped => AttemptOutcome::Failed(ImportFailure::ProgressUnreadable),
            Self::Stopped(reason) => AttemptOutcome::Failed(reason),
        }
    }
}

/// How a run ended and what it did before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
    pub end: RunEnd,
    pub counts: ImportCounts,
}

impl RunOutcome {
    pub fn attempt(self, started: Timestamp, ended: Timestamp) -> ImportAttempt {
        ImportAttempt {
            started,
            ended: Some(AttemptEnd {
                at: ended,
                outcome: self.end.outcome(),
                counts: self.counts,
            }),
        }
    }
}

impl<H: HealthProvider, P: Publisher, S: BackfillStore> HealthBackfill<'_, H, P, S> {
    /// Imports days until the range is done, `until` is near, or a step fails.
    /// `plan` turns the stored cursor, if any, into the range to work on, and
    /// the result is saved before the first fetch. A stop leaves the cursor on
    /// the first day not yet published.
    pub async fn run(
        &self,
        plan: impl FnOnce(Option<BackfillCursor>) -> BackfillCursor,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
        day_stamp: impl Fn(NaiveDate) -> Timestamp,
    ) -> RunOutcome {
        let mut counts = ImportCounts::default();
        let end = self.steps(plan, until, clock, day_stamp, &mut counts).await;
        RunOutcome { end, counts }
    }

    async fn steps(
        &self,
        plan: impl FnOnce(Option<BackfillCursor>) -> BackfillCursor,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
        day_stamp: impl Fn(NaiveDate) -> Timestamp,
        counts: &mut ImportCounts,
    ) -> RunEnd {
        let mut cursor = match self.cursor(plan) {
            Ok(cursor) => cursor,
            Err(end) => return end,
        };
        let mut opened = None;
        while !cursor.is_done() {
            let open = match open_session(self.provider, opened.take(), until, &clock).await {
                Ok(open) => open,
                Err(end) => return end,
            };
            if !turn(until, &clock).await {
                return RunEnd::OutOfTime;
            }
            if clock().duration_since(open.at) >= SESSION_MAX_AGE {
                continue;
            }
            let date = cursor.next;
            match self.day(&open.session, date, day_stamp(date)).await {
                Step::Stored => counts.stored += 1,
                Step::Empty => counts.empty += 1,
                Step::Skipped => counts.failed += 1,
                Step::Stop(reason) => return RunEnd::Stopped(reason),
            }
            let next = cursor.advanced();
            if let Err(error) = self.store.save(&next) {
                error!(%error, "cannot save the backfill cursor, stopping");
                return RunEnd::Stopped(ImportFailure::ProgressNotSaved);
            }
            cursor = next;
            opened = Some(open);
        }
        info!("health backfill finished");
        RunEnd::Done
    }

    /// The cursor to work from, or why there is nothing to do.
    fn cursor(
        &self,
        plan: impl FnOnce(Option<BackfillCursor>) -> BackfillCursor,
    ) -> Result<BackfillCursor, RunEnd> {
        let stored = self.store.load().map_err(|error| {
            error!(%error, "cannot read the backfill cursor, skipping the import");
            RunEnd::Skipped
        })?;
        let planned = plan(stored);
        if Some(planned) != stored {
            self.store.save(&planned).map_err(|error| {
                error!(%error, "cannot save the backfill cursor, skipping the import");
                RunEnd::Stopped(ImportFailure::ProgressNotSaved)
            })?;
            info!(next = %planned.next, last = %planned.last, "health backfill range saved");
        }
        if planned.is_done() {
            return Err(RunEnd::Done);
        }
        Ok(planned)
    }

    async fn day(&self, session: &H::Session, date: NaiveDate, stamp: Timestamp) -> Step {
        let health = match self.provider.daily(session, date).await {
            Ok(health) => health,
            Err(error) if H::is_session_error(&error) || H::is_throttled(&error) => {
                warn!(%error, "backfill stopped, resuming after the next pull");
                return Step::Stop(H::failure(&error));
            }
            Err(error) => {
                warn!(%error, %date, "backfill fetch failed, skipping this day");
                return Step::Skipped;
            }
        };
        self.publish(date, &health.measurements(self.source_id, stamp))
            .await
    }

    async fn publish(&self, date: NaiveDate, batch: &[Measurement]) -> Step {
        if batch.is_empty() {
            info!(%date, "backfill day empty, garmin returned no figures");
            return Step::Empty;
        }
        match self
            .publisher
            .publish(SourceKind::Health, self.source_id, batch)
            .await
        {
            Ok(()) => {
                info!(%date, metrics = batch.len(), "backfill day stored");
                Step::Stored
            }
            Err(error) => {
                warn!(%error, %date, "backfill publish failed, resuming after the next pull");
                Step::Stop(ImportFailure::PublishFailed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fmt;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::Duration;

    use chrono::{Datelike, NaiveDate};
    use domain::{
        AttemptOutcome, ImportCounts, ImportFailure, Measurement, SourceId, SourceKind, Timestamp,
    };
    use tokio::time::sleep;
    use tracing_subscriber::fmt::MakeWriter;

    use crate::test_archive::{MemoryBackfill, START, after, clock};
    use crate::{BackfillCursor, DailyHealth, HealthBackfill, HealthProvider, Publisher};

    use super::{RunEnd, RunOutcome};

    #[derive(Debug, Clone, Copy)]
    enum TestError {
        Session,
        Day,
        Throttled,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test error")
        }
    }

    impl std::error::Error for TestError {}

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    fn today() -> NaiveDate {
        date(2026, 11, 15)
    }

    fn jan(day: u32) -> NaiveDate {
        date(2026, 1, day)
    }

    fn stamp(date: NaiveDate) -> Timestamp {
        Timestamp::from_micros(i64::from(date.num_days_from_ce()))
    }

    fn range(cursor: BackfillCursor) -> Vec<NaiveDate> {
        cursor
            .next
            .iter_days()
            .take_while(|day| *day <= cursor.last)
            .collect()
    }

    fn year() -> Vec<NaiveDate> {
        range(BackfillCursor::anchored_at(today()))
    }

    fn steps() -> DailyHealth {
        DailyHealth {
            steps: Some(1),
            ..DailyHealth::default()
        }
    }

    #[derive(Default)]
    struct TestProvider {
        open_fails: bool,
        fetch_delay: Duration,
        results: HashMap<NaiveDate, Result<DailyHealth, TestError>>,
        opens: AtomicUsize,
        opens_at_fetch: Mutex<Vec<usize>>,
        fetched: Mutex<Vec<NaiveDate>>,
    }

    impl TestProvider {
        fn opens(&self) -> usize {
            self.opens.load(Ordering::SeqCst)
        }

        fn fetched(&self) -> Vec<NaiveDate> {
            self.fetched.lock().expect("lock").clone()
        }
    }

    impl HealthProvider for TestProvider {
        type Session = ();
        type Error = TestError;

        async fn open(&self) -> Result<(), TestError> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            if self.open_fails {
                Err(TestError::Session)
            } else {
                Ok(())
            }
        }

        async fn daily(&self, _session: &(), date: NaiveDate) -> Result<DailyHealth, TestError> {
            self.fetched.lock().expect("lock").push(date);
            self.opens_at_fetch.lock().expect("lock").push(self.opens());
            sleep(self.fetch_delay).await;
            self.results
                .get(&date)
                .copied()
                .unwrap_or_else(|| Ok(steps()))
        }

        fn is_session_error(error: &TestError) -> bool {
            matches!(error, TestError::Session)
        }

        fn is_throttled(error: &TestError) -> bool {
            matches!(error, TestError::Throttled)
        }
    }

    #[derive(Default)]
    struct TestPublisher {
        failing: HashSet<i64>,
        published: Mutex<Vec<(SourceKind, SourceId, Vec<Measurement>)>>,
    }

    impl TestPublisher {
        fn stamps(&self) -> Vec<i64> {
            self.published
                .lock()
                .expect("lock")
                .iter()
                .map(|(_, _, batch)| batch[0].timestamp.as_micros())
                .collect()
        }
    }

    impl Publisher for TestPublisher {
        type Error = io::Error;

        async fn publish(
            &self,
            source_kind: SourceKind,
            source_id: &SourceId,
            measurements: &[Measurement],
        ) -> Result<(), io::Error> {
            if measurements
                .iter()
                .any(|measurement| self.failing.contains(&measurement.timestamp.as_micros()))
            {
                return Err(io::Error::other("broker down"));
            }
            self.published.lock().expect("lock").push((
                source_kind,
                source_id.clone(),
                measurements.to_vec(),
            ));
            Ok(())
        }
    }

    const DAY: Duration = Duration::from_secs(86_400);

    async fn run_with(
        provider: &TestProvider,
        publisher: &TestPublisher,
        store: &MemoryBackfill,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
    ) -> RunOutcome {
        let source_id = SourceId::from("garmin".to_owned());
        HealthBackfill {
            provider,
            publisher,
            store,
            source_id: &source_id,
        }
        .run(
            |stored| stored.unwrap_or_else(|| BackfillCursor::anchored_at(today())),
            until,
            clock,
            stamp,
        )
        .await
    }

    async fn run(
        provider: &TestProvider,
        publisher: &TestPublisher,
        store: &MemoryBackfill,
        window: Duration,
    ) -> Timestamp {
        let clock = clock();
        run_with(provider, publisher, store, after(window), &clock).await;
        clock()
    }

    async fn end(
        provider: &TestProvider,
        publisher: &TestPublisher,
        store: &MemoryBackfill,
        window: Duration,
    ) -> RunEnd {
        run_with(provider, publisher, store, after(window), clock())
            .await
            .end
    }

    fn three_days() -> BackfillCursor {
        BackfillCursor {
            next: jan(1),
            last: jan(3),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_range_is_anchored_and_persisted_then_imported_to_the_end() {
        let (provider, publisher, store) = (
            TestProvider::default(),
            TestPublisher::default(),
            MemoryBackfill::default(),
        );
        run(&provider, &publisher, &store, DAY).await;
        let anchored = BackfillCursor::anchored_at(today());
        assert_eq!(store.saves()[0], anchored);
        assert_eq!(provider.fetched(), year());
        assert_eq!(year().len(), 364);
        assert!(store.stored().expect("cursor").is_done());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stored_cursor_wins_over_today() {
        let (provider, publisher) = (TestProvider::default(), TestPublisher::default());
        let store = MemoryBackfill::holding(three_days());
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.fetched(), [jan(1), jan(2), jan(3)]);
        assert_eq!(
            publisher.stamps(),
            [jan(1), jan(2), jan(3)].map(|day| stamp(day).as_micros())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_interrupted_import_resumes_without_repeating_a_day() {
        let (provider, publisher, store) = (
            TestProvider::default(),
            TestPublisher::default(),
            MemoryBackfill::default(),
        );
        run(&provider, &publisher, &store, Duration::from_secs(20 * 60)).await;
        let first = provider.fetched().len();
        assert!(0 < first && first < 364);
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.fetched(), year());
    }

    #[tokio::test(start_paused = true)]
    async fn the_cursor_advances_once_per_published_day() {
        let (provider, publisher) = (TestProvider::default(), TestPublisher::default());
        let store = MemoryBackfill::holding(three_days());
        run(&provider, &publisher, &store, DAY).await;
        let saved: Vec<_> = store.saves().iter().map(|cursor| cursor.next).collect();
        assert_eq!(saved, [jan(2), jan(3), jan(4)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_publish_stops_without_advancing_and_the_day_is_fetched_again() {
        let provider = TestProvider::default();
        let failing = TestPublisher {
            failing: [stamp(jan(2)).as_micros()].into(),
            ..TestPublisher::default()
        };
        let store = MemoryBackfill::holding(three_days());
        run(&provider, &failing, &store, DAY).await;
        assert_eq!(provider.fetched(), [jan(1), jan(2)]);
        assert_eq!(store.stored().expect("cursor").next, jan(2));
        let working = TestPublisher::default();
        run(&provider, &working, &store, DAY).await;
        assert_eq!(provider.fetched(), [jan(1), jan(2), jan(2), jan(3)]);
        assert!(store.stored().expect("cursor").is_done());
    }

    #[tokio::test(start_paused = true)]
    async fn throttling_stops_the_import_and_the_next_run_resumes_at_that_day() {
        let throttled = TestProvider {
            results: HashMap::from([(jan(2), Err(TestError::Throttled))]),
            ..TestProvider::default()
        };
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill::holding(three_days()),
        );
        run(&throttled, &publisher, &store, DAY).await;
        assert_eq!(throttled.fetched(), [jan(1), jan(2)]);
        assert_eq!(store.stored().expect("cursor").next, jan(2));
        let calm = TestProvider::default();
        run(&calm, &publisher, &store, DAY).await;
        assert_eq!(calm.fetched(), [jan(2), jan(3)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_open_stops_before_any_fetch_with_the_range_saved() {
        let provider = TestProvider {
            open_fails: true,
            ..TestProvider::default()
        };
        let (publisher, store) = (TestPublisher::default(), MemoryBackfill::default());
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.opens(), 1);
        assert!(provider.fetched().is_empty());
        assert_eq!(store.saves(), [BackfillCursor::anchored_at(today())]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_error_on_a_day_stops_and_a_later_run_resumes() {
        let rejected = TestProvider {
            results: HashMap::from([(jan(2), Err(TestError::Session))]),
            ..TestProvider::default()
        };
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill::holding(three_days()),
        );
        run(&rejected, &publisher, &store, DAY).await;
        assert_eq!(store.stored().expect("cursor").next, jan(2));
        let healthy = TestProvider::default();
        run(&healthy, &publisher, &store, DAY).await;
        assert!(store.stored().expect("cursor").is_done());
    }

    #[tokio::test(start_paused = true)]
    async fn the_daily_pull_is_not_delayed() {
        let provider = TestProvider {
            fetch_delay: Duration::from_secs(6),
            ..TestProvider::default()
        };
        let (publisher, store) = (TestPublisher::default(), MemoryBackfill::default());
        let window = Duration::from_secs(20 * 60);
        let end = run(&provider, &publisher, &store, window).await;
        assert!(!provider.fetched().is_empty());
        assert!(end <= after(window));
    }

    #[tokio::test(start_paused = true)]
    async fn no_step_starts_without_room_for_the_spacing_and_the_margin() {
        let provider = TestProvider::default();
        let (publisher, store) = (TestPublisher::default(), MemoryBackfill::default());
        run(
            &provider,
            &publisher,
            &store,
            Duration::from_secs(6 * 60 - 1),
        )
        .await;
        assert_eq!(provider.opens(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_wake_past_the_window_starts_nothing() {
        let provider = TestProvider::default();
        let (publisher, store) = (TestPublisher::default(), MemoryBackfill::default());
        let calls = AtomicUsize::new(0);
        let late_wake = || {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Timestamp::from_micros(START)
            } else {
                after(Duration::from_secs(2 * 3600))
            }
        };
        let until = after(Duration::from_secs(3600));
        run_with(&provider, &publisher, &store, until, late_wake).await;
        assert_eq!(provider.opens(), 0);
        assert!(provider.fetched().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_import_never_contacts_garmin_again() {
        let provider = TestProvider::default();
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill::holding(three_days()),
        );
        run(&provider, &publisher, &store, DAY).await;
        let opens = provider.opens();
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.opens(), opens);
        assert_eq!(provider.fetched().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_is_reused_within_its_age() {
        let provider = TestProvider::default();
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill::holding(three_days()),
        );
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.opens(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_is_replaced_after_thirty_minutes_whatever_the_day_count() {
        let provider = TestProvider {
            fetch_delay: Duration::from_secs(120),
            ..TestProvider::default()
        };
        let publisher = TestPublisher::default();
        let store = MemoryBackfill::holding(BackfillCursor {
            next: jan(1),
            last: jan(12),
        });
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.fetched().len(), 12);
        assert_eq!(provider.opens(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_session_age_is_checked_after_the_pacing_wait() {
        let provider = TestProvider::default();
        let publisher = TestPublisher::default();
        let store = MemoryBackfill::holding(BackfillCursor {
            next: jan(1),
            last: jan(31),
        });
        run(&provider, &publisher, &store, DAY).await;
        let seen = provider.opens_at_fetch.lock().expect("lock").clone();
        assert_eq!(seen[28], 1);
        assert_eq!(seen[29], 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_day_after_an_open_waits_and_needs_the_margin() {
        let provider = TestProvider::default();
        let (publisher, store) = (TestPublisher::default(), MemoryBackfill::default());
        run(
            &provider,
            &publisher,
            &store,
            Duration::from_secs(6 * 60 + 30),
        )
        .await;
        assert_eq!(provider.opens(), 1);
        assert!(provider.fetched().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn failed_and_empty_days_are_passed_over() {
        let provider = TestProvider {
            results: HashMap::from([
                (jan(1), Err(TestError::Day)),
                (jan(2), Ok(DailyHealth::default())),
            ]),
            ..TestProvider::default()
        };
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill::holding(three_days()),
        );
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(publisher.stamps(), [stamp(jan(3)).as_micros()]);
        assert!(store.stored().expect("cursor").is_done());
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl MakeWriter<'_> for Captured {
        type Writer = Self;

        fn make_writer(&self) -> Self {
            self.clone()
        }
    }

    /// A global subscriber: a scoped one races with callsites that other
    /// tests already registered with no subscriber.
    fn capture() -> &'static Captured {
        static CAPTURED: LazyLock<Captured> = LazyLock::new(|| {
            let captured = Captured::default();
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(captured.clone())
                .init();
            captured
        });
        &CAPTURED
    }

    #[tokio::test(start_paused = true)]
    async fn each_passed_day_logs_one_info_line_and_a_failed_day_keeps_its_warning() {
        let captured = capture();
        let march = |day| date(2025, 3, day);
        let provider = TestProvider {
            results: HashMap::from([
                (
                    march(1),
                    Ok(DailyHealth {
                        steps: Some(987_654),
                        ..DailyHealth::default()
                    }),
                ),
                (march(2), Ok(DailyHealth::default())),
                (march(3), Err(TestError::Day)),
            ]),
            ..TestProvider::default()
        };
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill::holding(BackfillCursor {
                next: march(1),
                last: march(3),
            }),
        );
        run(&provider, &publisher, &store, DAY).await;
        let logs = String::from_utf8(captured.0.lock().expect("lock").clone()).expect("utf8");
        let lines = |date: NaiveDate| {
            logs.lines()
                .filter(|line| line.contains(&format!("date={date}")))
                .collect::<Vec<_>>()
        };
        let [stored] = lines(march(1))[..] else {
            panic!("one line for the stored day: {logs}")
        };
        assert!(stored.contains("INFO") && stored.contains("backfill day stored"));
        assert!(stored.contains("metrics=1"));
        let [empty] = lines(march(2))[..] else {
            panic!("one line for the empty day: {logs}")
        };
        assert!(empty.contains("INFO") && empty.contains("garmin returned no figures"));
        let [failed] = lines(march(3))[..] else {
            panic!("one line for the failed day: {logs}")
        };
        assert!(failed.contains("WARN") && !failed.contains("INFO"));
        assert!(!logs.contains("987654"));
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_cursor_opens_and_saves_nothing() {
        let provider = TestProvider::default();
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill {
                load_fails: true,
                ..MemoryBackfill::default()
            },
        );
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.opens(), 0);
        assert!(store.saves().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_save_stops_after_the_first_day() {
        let provider = TestProvider::default();
        let publisher = TestPublisher::default();
        let store = MemoryBackfill {
            save_fails: true,
            stored: Mutex::new(Some(three_days())),
            ..MemoryBackfill::default()
        };
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.fetched(), [jan(1)]);
        assert_eq!(publisher.stamps().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_anchor_save_opens_nothing() {
        let provider = TestProvider::default();
        let (publisher, store) = (
            TestPublisher::default(),
            MemoryBackfill {
                save_fails: true,
                ..MemoryBackfill::default()
            },
        );
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.opens(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unchanged_plan_saves_nothing_and_a_changed_one_saves_once() {
        let (provider, publisher) = (TestProvider::default(), TestPublisher::default());
        let store = MemoryBackfill::holding(three_days());
        let source_id = SourceId::from("garmin".to_owned());
        let backfill = HealthBackfill {
            provider: &provider,
            publisher: &publisher,
            store: &store,
            source_id: &source_id,
        };
        let planned = BackfillCursor {
            next: jan(3),
            last: jan(3),
        };
        backfill
            .run(|_| three_days(), after(Duration::ZERO), clock(), stamp)
            .await;
        assert!(store.saves().is_empty());
        backfill.run(|_| planned, after(DAY), clock(), stamp).await;
        assert_eq!(store.saves()[0], planned);
        assert_eq!(provider.fetched(), [jan(3)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_plan_that_is_done_is_saved_and_ends_done_without_contacting_garmin() {
        let (provider, publisher) = (TestProvider::default(), TestPublisher::default());
        let store = MemoryBackfill::holding(three_days());
        let source_id = SourceId::from("garmin".to_owned());
        let done = BackfillCursor {
            next: jan(5),
            last: jan(4),
        };
        let end = HealthBackfill {
            provider: &provider,
            publisher: &publisher,
            store: &store,
            source_id: &source_id,
        }
        .run(|_| done, after(DAY), clock(), stamp)
        .await
        .end;
        assert_eq!(end, RunEnd::Done);
        assert_eq!(store.stored(), Some(done));
        assert_eq!(provider.opens(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_range_ends_done() {
        let (provider, publisher) = (TestProvider::default(), TestPublisher::default());
        let store = MemoryBackfill::holding(three_days());
        assert_eq!(end(&provider, &publisher, &store, DAY).await, RunEnd::Done);
        assert_eq!(end(&provider, &publisher, &store, DAY).await, RunEnd::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn a_short_window_ends_out_of_time() {
        let (provider, publisher) = (TestProvider::default(), TestPublisher::default());
        let store = MemoryBackfill::holding(three_days());
        let window = Duration::from_secs(6 * 60 - 1);
        assert_eq!(
            end(&provider, &publisher, &store, window).await,
            RunEnd::OutOfTime
        );
        let window = Duration::from_secs(20 * 60);
        let store = MemoryBackfill::default();
        assert_eq!(
            end(&provider, &publisher, &store, window).await,
            RunEnd::OutOfTime
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_cursor_ends_skipped() {
        let (provider, publisher) = (TestProvider::default(), TestPublisher::default());
        let store = MemoryBackfill {
            load_fails: true,
            ..MemoryBackfill::default()
        };
        assert_eq!(
            end(&provider, &publisher, &store, DAY).await,
            RunEnd::Skipped
        );
    }

    fn stopped(reason: ImportFailure) -> RunEnd {
        RunEnd::Stopped(reason)
    }

    #[tokio::test(start_paused = true)]
    async fn failures_end_stopped_with_their_reason_and_the_cursor_stays() {
        let publisher = TestPublisher::default();
        let store = || MemoryBackfill::holding(three_days());
        let open_fails = TestProvider {
            open_fails: true,
            ..TestProvider::default()
        };
        let session = TestProvider {
            results: HashMap::from([(jan(1), Err(TestError::Session))]),
            ..TestProvider::default()
        };
        let throttled = TestProvider {
            results: HashMap::from([(jan(1), Err(TestError::Throttled))]),
            ..TestProvider::default()
        };
        for (provider, reason) in [
            (open_fails, ImportFailure::LoginRejected),
            (session, ImportFailure::LoginRejected),
            (throttled, ImportFailure::Throttled),
        ] {
            let store = store();
            assert_eq!(
                end(&provider, &publisher, &store, DAY).await,
                stopped(reason)
            );
            assert_eq!(store.stored(), Some(three_days()));
        }
        let failing = TestPublisher {
            failing: [stamp(jan(1)).as_micros()].into(),
            ..TestPublisher::default()
        };
        let provider = TestProvider::default();
        let unpublished = store();
        assert_eq!(
            end(&provider, &failing, &unpublished, DAY).await,
            stopped(ImportFailure::PublishFailed)
        );
        assert_eq!(unpublished.stored(), Some(three_days()));
        let unsaved = MemoryBackfill {
            save_fails: true,
            stored: Mutex::new(Some(three_days())),
            ..MemoryBackfill::default()
        };
        assert_eq!(
            end(&provider, &publisher, &unsaved, DAY).await,
            stopped(ImportFailure::ProgressNotSaved)
        );
        assert_eq!(unsaved.stored(), Some(three_days()));
        let unanchored = MemoryBackfill {
            save_fails: true,
            ..MemoryBackfill::default()
        };
        assert_eq!(
            end(&provider, &publisher, &unanchored, DAY).await,
            stopped(ImportFailure::ProgressNotSaved)
        );
        assert_eq!(unanchored.stored(), None);
    }

    async fn outcome(
        provider: &TestProvider,
        publisher: &TestPublisher,
        store: &MemoryBackfill,
    ) -> RunOutcome {
        run_with(provider, publisher, store, after(DAY), clock()).await
    }

    fn counts(stored: u32, empty: u32, failed: u32) -> ImportCounts {
        ImportCounts {
            stored,
            empty,
            failed,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn counts_follow_the_outcome_of_each_day() {
        let provider = TestProvider {
            results: HashMap::from([
                (jan(1), Err(TestError::Day)),
                (jan(2), Ok(DailyHealth::default())),
            ]),
            ..TestProvider::default()
        };
        let publisher = TestPublisher::default();
        let store = MemoryBackfill::holding(three_days());
        let done = outcome(&provider, &publisher, &store).await;
        assert_eq!(done.end, RunEnd::Done);
        assert_eq!(done.counts, counts(1, 1, 1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_keeps_the_days_already_counted() {
        let provider = TestProvider {
            results: HashMap::from([(jan(2), Err(TestError::Throttled))]),
            ..TestProvider::default()
        };
        let store = MemoryBackfill::holding(three_days());
        let stopped = outcome(&provider, &TestPublisher::default(), &store).await;
        assert_eq!(stopped.end, RunEnd::Stopped(ImportFailure::Throttled));
        assert_eq!(stopped.counts, counts(1, 0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_cursor_is_a_skip_with_no_counts() {
        let store = MemoryBackfill {
            load_fails: true,
            ..MemoryBackfill::default()
        };
        let skipped = outcome(&TestProvider::default(), &TestPublisher::default(), &store).await;
        assert_eq!(skipped.end, RunEnd::Skipped);
        assert_eq!(skipped.counts, ImportCounts::default());
    }

    #[test]
    fn an_end_reads_as_an_attempt_outcome() {
        for (end, outcome) in [
            (RunEnd::Done, AttemptOutcome::Done),
            (RunEnd::OutOfTime, AttemptOutcome::Paused),
            (
                RunEnd::Skipped,
                AttemptOutcome::Failed(ImportFailure::ProgressUnreadable),
            ),
            (
                RunEnd::Stopped(ImportFailure::GarminDown),
                AttemptOutcome::Failed(ImportFailure::GarminDown),
            ),
        ] {
            assert_eq!(end.outcome(), outcome);
        }
    }

    #[test]
    fn an_attempt_carries_the_outcome_and_counts() {
        let run = RunOutcome {
            end: RunEnd::OutOfTime,
            counts: counts(2, 0, 1),
        };
        let attempt = run.attempt(Timestamp::from_micros(1), Timestamp::from_micros(2));
        assert_eq!(attempt.started, Timestamp::from_micros(1));
        let ended = attempt.ended.expect("ended");
        assert_eq!(ended.at, Timestamp::from_micros(2));
        assert_eq!(ended.outcome, AttemptOutcome::Paused);
        assert_eq!(ended.counts, counts(2, 0, 1));
    }
}
