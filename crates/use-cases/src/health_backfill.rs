use std::time::Duration;

use chrono::NaiveDate;
use domain::{Measurement, SourceId, SourceKind, Timestamp};
use tokio::time::sleep;
use tracing::{error, info, warn};

use crate::{BackfillCursor, BackfillStore, HealthProvider, Publisher};

/// Wait before every Garmin step, a session open or one day, to stay far
/// below the rate that gets clients blocked.
const DAY_SPACING: Duration = Duration::from_secs(60);
/// Time left before the deadline that a step must not eat into.
const MARGIN: Duration = Duration::from_secs(5 * 60);
/// A session is replaced after this long, well within the token's hour.
const SESSION_MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// Imports the past year of health days one at a time, resuming from a
/// persisted cursor, and stops before `until` so the daily pull is not delayed.
pub struct HealthBackfill<'a, H, P, S> {
    pub provider: &'a H,
    pub publisher: &'a P,
    pub store: &'a S,
    pub source_id: &'a SourceId,
}

struct Opened<T> {
    session: T,
    at: Timestamp,
}

enum Step {
    Next,
    Stop,
}

impl<H: HealthProvider, P: Publisher, S: BackfillStore> HealthBackfill<'_, H, P, S> {
    /// Imports days until the range is done, `until` is near, or a step fails.
    /// A stop leaves the cursor on the first day not yet published.
    pub async fn run(
        &self,
        today: NaiveDate,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
        day_stamp: impl Fn(NaiveDate) -> Timestamp,
    ) {
        let Some(mut cursor) = self.cursor(today) else {
            return;
        };
        let mut opened = None;
        while !cursor.is_done() {
            let Some(open) = self.session(opened.take(), until, &clock).await else {
                return;
            };
            if !turn(until, &clock).await {
                return;
            }
            if clock().duration_since(open.at) >= SESSION_MAX_AGE {
                continue;
            }
            let date = cursor.next;
            if let Step::Stop = self.day(&open.session, date, day_stamp(date)).await {
                return;
            }
            let next = cursor.advanced();
            if let Err(error) = self.store.save(&next) {
                error!(%error, "cannot save the backfill cursor, stopping");
                return;
            }
            cursor = next;
            opened = Some(open);
        }
        info!("health backfill finished");
    }

    /// The cursor to work from; `None` when there is nothing to do.
    fn cursor(&self, today: NaiveDate) -> Option<BackfillCursor> {
        let cursor = match self.store.load() {
            Ok(Some(cursor)) => cursor,
            Ok(None) => self.anchor(today)?,
            Err(error) => {
                error!(%error, "cannot read the backfill cursor, skipping the import");
                return None;
            }
        };
        (!cursor.is_done()).then_some(cursor)
    }

    fn anchor(&self, today: NaiveDate) -> Option<BackfillCursor> {
        let cursor = BackfillCursor::anchored_at(today);
        if let Err(error) = self.store.save(&cursor) {
            error!(%error, "cannot save the backfill cursor, skipping the import");
            return None;
        }
        info!(next = %cursor.next, last = %cursor.last, "health backfill range saved");
        Some(cursor)
    }

    /// `current`, else a new session after its own wait; `None` when there is
    /// no time left or Garmin refuses the session.
    async fn session(
        &self,
        current: Option<Opened<H::Session>>,
        until: Timestamp,
        clock: &impl Fn() -> Timestamp,
    ) -> Option<Opened<H::Session>> {
        if current.is_some() {
            return current;
        }
        if !turn(until, clock).await {
            return None;
        }
        match self.provider.open().await {
            Ok(session) => Some(Opened {
                session,
                at: clock(),
            }),
            Err(error) => {
                warn!(%error, "backfill session failed, resuming after the next pull");
                None
            }
        }
    }

    async fn day(&self, session: &H::Session, date: NaiveDate, stamp: Timestamp) -> Step {
        let health = match self.provider.daily(session, date).await {
            Ok(health) => health,
            Err(error) if H::is_session_error(&error) || H::is_throttled(&error) => {
                warn!(%error, "backfill stopped, resuming after the next pull");
                return Step::Stop;
            }
            Err(error) => {
                warn!(%error, %date, "backfill fetch failed, skipping this day");
                return Step::Next;
            }
        };
        self.publish(date, &health.measurements(self.source_id, stamp))
            .await
    }

    async fn publish(&self, date: NaiveDate, batch: &[Measurement]) -> Step {
        if batch.is_empty() {
            return Step::Next;
        }
        match self
            .publisher
            .publish(SourceKind::Health, self.source_id, batch)
            .await
        {
            Ok(()) => Step::Next,
            Err(error) => {
                warn!(%error, %date, "backfill publish failed, resuming after the next pull");
                Step::Stop
            }
        }
    }
}

/// Waits `DAY_SPACING` if a step fits before `until`, checking again after the
/// wait so a late wake starts nothing.
async fn turn(until: Timestamp, clock: &impl Fn() -> Timestamp) -> bool {
    if until.duration_since(clock()) < DAY_SPACING + MARGIN {
        return false;
    }
    sleep(DAY_SPACING).await;
    until.duration_since(clock()) >= MARGIN
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fmt;
    use std::io;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use chrono::{Datelike, NaiveDate};
    use domain::{Measurement, SourceId, SourceKind, Timestamp};
    use tokio::time::{Instant, sleep};

    use crate::{
        BackfillCursor, BackfillStore, DailyHealth, HealthBackfill, HealthProvider, Publisher,
    };

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

    #[derive(Default)]
    struct TestStore {
        load_fails: bool,
        save_fails: bool,
        stored: Mutex<Option<BackfillCursor>>,
        saves: Mutex<Vec<BackfillCursor>>,
    }

    impl TestStore {
        fn holding(cursor: BackfillCursor) -> Self {
            Self {
                stored: Mutex::new(Some(cursor)),
                ..Self::default()
            }
        }

        fn stored(&self) -> Option<BackfillCursor> {
            *self.stored.lock().expect("lock")
        }

        fn saves(&self) -> Vec<BackfillCursor> {
            self.saves.lock().expect("lock").clone()
        }
    }

    impl BackfillStore for TestStore {
        type Error = io::Error;

        fn load(&self) -> Result<Option<BackfillCursor>, io::Error> {
            if self.load_fails {
                return Err(io::Error::other("corrupt"));
            }
            Ok(self.stored())
        }

        fn save(&self, cursor: &BackfillCursor) -> Result<(), io::Error> {
            if self.save_fails {
                return Err(io::Error::other("disk full"));
            }
            self.saves.lock().expect("lock").push(*cursor);
            *self.stored.lock().expect("lock") = Some(*cursor);
            Ok(())
        }
    }

    const START: i64 = 1_000_000_000_000_000;

    fn clock() -> impl Fn() -> Timestamp {
        let start = Instant::now();
        move || {
            let elapsed = i64::try_from(start.elapsed().as_micros()).expect("fits");
            Timestamp::from_micros(START + elapsed)
        }
    }

    fn after(window: Duration) -> Timestamp {
        let window = i64::try_from(window.as_micros()).expect("fits");
        Timestamp::from_micros(START + window)
    }

    const DAY: Duration = Duration::from_secs(86_400);

    async fn run_with(
        provider: &TestProvider,
        publisher: &TestPublisher,
        store: &TestStore,
        until: Timestamp,
        clock: impl Fn() -> Timestamp,
    ) {
        let source_id = SourceId::from("garmin".to_owned());
        HealthBackfill {
            provider,
            publisher,
            store,
            source_id: &source_id,
        }
        .run(today(), until, clock, stamp)
        .await;
    }

    async fn run(
        provider: &TestProvider,
        publisher: &TestPublisher,
        store: &TestStore,
        window: Duration,
    ) -> Timestamp {
        let clock = clock();
        run_with(provider, publisher, store, after(window), &clock).await;
        clock()
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
            TestStore::default(),
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
        let store = TestStore::holding(three_days());
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
            TestStore::default(),
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
        let store = TestStore::holding(three_days());
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
        let store = TestStore::holding(three_days());
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
        let (publisher, store) = (TestPublisher::default(), TestStore::holding(three_days()));
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
        let (publisher, store) = (TestPublisher::default(), TestStore::default());
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
        let (publisher, store) = (TestPublisher::default(), TestStore::holding(three_days()));
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
        let (publisher, store) = (TestPublisher::default(), TestStore::default());
        let window = Duration::from_secs(20 * 60);
        let end = run(&provider, &publisher, &store, window).await;
        assert!(!provider.fetched().is_empty());
        assert!(end <= after(window));
    }

    #[tokio::test(start_paused = true)]
    async fn no_step_starts_without_room_for_the_spacing_and_the_margin() {
        let provider = TestProvider::default();
        let (publisher, store) = (TestPublisher::default(), TestStore::default());
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
        let (publisher, store) = (TestPublisher::default(), TestStore::default());
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
        let (publisher, store) = (TestPublisher::default(), TestStore::holding(three_days()));
        run(&provider, &publisher, &store, DAY).await;
        let opens = provider.opens();
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.opens(), opens);
        assert_eq!(provider.fetched().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_is_reused_within_its_age() {
        let provider = TestProvider::default();
        let (publisher, store) = (TestPublisher::default(), TestStore::holding(three_days()));
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
        let store = TestStore::holding(BackfillCursor {
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
        let store = TestStore::holding(BackfillCursor {
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
        let (publisher, store) = (TestPublisher::default(), TestStore::default());
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
        let (publisher, store) = (TestPublisher::default(), TestStore::holding(three_days()));
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(publisher.stamps(), [stamp(jan(3)).as_micros()]);
        assert!(store.stored().expect("cursor").is_done());
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_cursor_opens_and_saves_nothing() {
        let provider = TestProvider::default();
        let (publisher, store) = (
            TestPublisher::default(),
            TestStore {
                load_fails: true,
                ..TestStore::default()
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
        let store = TestStore {
            save_fails: true,
            stored: Mutex::new(Some(three_days())),
            ..TestStore::default()
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
            TestStore {
                save_fails: true,
                ..TestStore::default()
            },
        );
        run(&provider, &publisher, &store, DAY).await;
        assert_eq!(provider.opens(), 0);
    }
}
