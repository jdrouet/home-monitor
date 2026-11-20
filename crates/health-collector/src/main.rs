mod command;
mod import_view;
mod prompt;
mod schedule;

use std::convert::Infallible;
use std::env;
use std::io::{self, BufRead, Write};

use anyhow::{Context, anyhow};
use command::Command;
use config::{HealthCollectorConfig, MqttConfig};
use domain::{GARMIN, ImportAttempt, ImportJob, ImportStatus, SourceId, Timestamp};
use garmin_connect::{GarminConnect, TokenFile};
use import_view::ImportView;
use mqtt::MqttPublisher;
use paris_clock::paris_date;
use prompt::prompt_line;
use schedule::{completed_day_stamp, next_pull, pull_days, wait_until};
use state_file::{ActivityCursorFile, ArchiveDir, BackfillFile};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::sleep;
use use_cases::{
    ActivityCursorStore, ActivityImport, ArchiveDays, ArchiveSource, BackfillStore, HealthBackfill,
    HealthImport, ImportCursors, Publisher, PullWindow, RawArchive, collect_health, report_session,
};

const CLIENT_ID: &str = "health-collector";
const USAGE: &str = "usage: health-collector [login]";
const GAP_FILE: &str = "garmin_gap.json";
const BACKFILL_FILE: &str = "garmin_backfill.json";
const ACTIVITIES_FILE: &str = "garmin_activities.json";
const ARCHIVE_FILE: &str = "garmin_archive.json";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let command = Command::parse(env::args().skip(1)).ok_or_else(|| anyhow!(USAGE))?;
    let config = config::load()?;
    let health = config
        .health_collector
        .as_ref()
        .context("config has no [health_collector] section")?;
    dispatch(command, &config.mqtt, health).await
}

async fn dispatch(
    command: Command,
    mqtt: &MqttConfig,
    health: &HealthCollectorConfig,
) -> anyhow::Result<()> {
    match command {
        Command::Login => login(health).await,
        Command::Collect => collect(mqtt, health).await,
    }
}

async fn login(health: &HealthCollectorConfig) -> anyhow::Result<()> {
    let garmin = GarminConnect::new(TokenFile::new(health.token_file.clone()))?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout();
    let (email, password) = read_credentials(&mut input, &mut output)?;
    garmin
        .login(&email, &password, || {
            prompt_line(&mut input, &mut output, "MFA code")
        })
        .await?;
    println!("Logged in; the tokens are saved.");
    Ok(())
}

fn read_credentials(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<(String, String)> {
    let email = prompt_line(input, output, "Garmin email")?;
    let password = rpassword::prompt_password("Garmin password: ")?;
    Ok((email, password))
}

async fn collect(mqtt: &MqttConfig, health: &HealthCollectorConfig) -> anyhow::Result<()> {
    let listener = TcpListener::bind(health.http_listen).await?;
    let provider = GarminConnect::new(TokenFile::new(health.token_file.clone()))?;
    let publisher = MqttPublisher::connect(&mqtt.host, mqtt.port, CLIENT_ID);
    let cursor_file = |name: &str| BackfillFile::new(health.token_file.with_file_name(name));
    let activity_file =
        || ActivityCursorFile::new(health.token_file.with_file_name(ACTIVITIES_FILE));
    let backfill_store = cursor_file(BACKFILL_FILE);
    let gap_store = cursor_file(GAP_FILE);
    let archive = ArchiveDir::new(health.archive_dir.clone());
    let activity_store = activity_file();
    let archive_store = cursor_file(ARCHIVE_FILE);
    let (jobs, watched) = watch::channel(ImportStatus::default());
    let router = import_view::router(ImportView {
        jobs: watched,
        cursors: ImportCursors {
            gap: cursor_file(GAP_FILE),
            yearly: cursor_file(BACKFILL_FILE),
            activities: activity_file(),
            archive: cursor_file(ARCHIVE_FILE),
        },
    });
    let source_id = SourceId::from(GARMIN.to_owned());
    let import = HealthImport {
        gap: HealthBackfill {
            provider: &provider,
            publisher: &publisher,
            store: &gap_store,
            source_id: &source_id,
        },
        yearly: HealthBackfill {
            provider: &provider,
            publisher: &publisher,
            store: &backfill_store,
            source_id: &source_id,
        },
        activities: ActivityImport {
            source: &provider,
            archive: &archive,
            store: &activity_store,
        },
        archive_days: ArchiveDays {
            source: &provider,
            archive: &archive,
            store: &archive_store,
        },
    };
    tokio::select! {
        served = axum::serve(listener, router) => Err(anyhow!("health server ended: {served:?}")),
        never = pull_forever(&provider, &publisher, &import, &source_id, &jobs) => match never {},
    }
}

/// Pulls at startup, then at every daily pull time. The next deadline is
/// fixed before a pull starts, so a slow pull cannot skip it. A failed pull
/// is not retried before the next one.
async fn pull_forever<
    H: ArchiveSource,
    P: Publisher,
    A: RawArchive,
    S: BackfillStore,
    C: ActivityCursorStore,
>(
    provider: &H,
    publisher: &P,
    import: &HealthImport<'_, H, P, A, S, C>,
    source_id: &SourceId,
    jobs: &watch::Sender<ImportStatus>,
) -> Infallible {
    loop {
        let next = pull_once(
            provider,
            publisher,
            import,
            source_id,
            jobs,
            Timestamp::now(),
            Timestamp::now,
        )
        .await;
        sleep(wait_until(next, Timestamp::now())).await;
    }
}

/// One pull started at `started`, reporting the session status, and after a
/// completed pull the imports until shortly before the next deadline, which it
/// returns. Every job's record carries that deadline as the pull it waits for.
async fn pull_once<
    H: ArchiveSource,
    P: Publisher,
    A: RawArchive,
    S: BackfillStore,
    C: ActivityCursorStore,
>(
    provider: &H,
    publisher: &P,
    import: &HealthImport<'_, H, P, A, S, C>,
    source_id: &SourceId,
    jobs: &watch::Sender<ImportStatus>,
    started: Timestamp,
    clock: impl Fn() -> Timestamp,
) -> Timestamp {
    let next = next_pull(started);
    let days = pull_days(started);
    jobs.send_modify(|status| status.next_pull = Some(next));
    let record = |job: ImportJob, attempt: ImportAttempt| {
        jobs.send_modify(|status| status.record(job, attempt, next));
    };
    record(
        ImportJob::DailyPull,
        ImportAttempt {
            started,
            ended: None,
        },
    );
    let pull = collect_health(provider, publisher, source_id, &days).await;
    report_session(publisher, source_id, &pull, started).await;
    record(ImportJob::DailyPull, pull.attempt(started, clock()));
    import
        .after_pull(
            &pull,
            &days,
            PullWindow {
                today: paris_date(started),
                until: next,
                clock: &clock,
                day_stamp: completed_day_stamp,
            },
            record,
        )
        .await;
    next
}

#[cfg(test)]
mod tests {
    use std::io;

    use chrono::{DateTime, NaiveDate};
    use domain::{
        AttemptOutcome, ImportAttempt, ImportFailure, ImportJob, ImportStatus, Measurement,
        SourceId, SourceKind, Timestamp,
    };
    use tokio::sync::watch;
    use use_cases::{
        ActivityCursor, ActivityCursorStore, ActivityImport, ActivityRef, ArchiveDays, ArchiveKey,
        ArchiveSource, BackfillCursor, BackfillStore, DailyHealth, DocumentKey, Fetched,
        HealthBackfill, HealthImport, HealthProvider, Publisher, RawArchive,
    };

    use super::pull_once;

    /// Garmin is down, and nothing else is reachable.
    struct Down;

    impl HealthProvider for Down {
        type Session = ();
        type Error = io::Error;

        async fn open(&self) -> Result<(), io::Error> {
            Err(io::Error::other("down"))
        }

        async fn daily(&self, _session: &(), _date: NaiveDate) -> Result<DailyHealth, io::Error> {
            Err(io::Error::other("down"))
        }

        fn is_session_error(_error: &io::Error) -> bool {
            false
        }

        fn is_throttled(_error: &io::Error) -> bool {
            false
        }
    }

    impl ArchiveSource for Down {
        type Document = ();

        const DAILY: &'static [()] = &[];
        const ACTIVITY: &'static [()] = &[];

        fn file_name(_document: ()) -> &'static str {
            ""
        }

        async fn document(
            &self,
            _session: &(),
            _document: (),
            _key: DocumentKey,
        ) -> Result<Fetched, io::Error> {
            Err(io::Error::other("down"))
        }

        async fn activities(
            &self,
            _session: &(),
            _start: u32,
            _limit: u32,
        ) -> Result<Vec<ActivityRef>, io::Error> {
            Err(io::Error::other("down"))
        }
    }

    impl Publisher for Down {
        type Error = io::Error;

        async fn publish(
            &self,
            _source_kind: SourceKind,
            _source_id: &SourceId,
            _measurements: &[Measurement],
        ) -> Result<(), io::Error> {
            Ok(())
        }
    }

    impl RawArchive for Down {
        type Error = io::Error;

        fn contains(&self, _key: &ArchiveKey) -> Result<bool, io::Error> {
            Ok(false)
        }

        fn put(&self, _key: &ArchiveKey, _bytes: &[u8]) -> Result<(), io::Error> {
            Ok(())
        }

        fn put_denied(&self, _key: &ArchiveKey) -> Result<(), io::Error> {
            Ok(())
        }
    }

    impl BackfillStore for Down {
        type Error = io::Error;

        fn load(&self) -> Result<Option<BackfillCursor>, io::Error> {
            Ok(None)
        }

        fn save(&self, _cursor: &BackfillCursor) -> Result<(), io::Error> {
            Ok(())
        }
    }

    impl ActivityCursorStore for Down {
        type Error = io::Error;

        fn load(&self) -> Result<Option<ActivityCursor>, io::Error> {
            Ok(None)
        }

        fn save(&self, _cursor: &ActivityCursor) -> Result<(), io::Error> {
            Ok(())
        }
    }

    fn at(rfc3339: &str) -> Timestamp {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid instant");
        Timestamp::from_micros(instant.timestamp_micros())
    }

    fn after(start: Timestamp, seconds: i64) -> Timestamp {
        Timestamp::from_micros(start.as_micros() + seconds * 1_000_000)
    }

    /// A pull started at `started` that ends at `ends`.
    async fn pull(
        jobs: &watch::Sender<ImportStatus>,
        started: Timestamp,
        ends: Timestamp,
    ) -> Timestamp {
        let source_id = SourceId::from("garmin".to_owned());
        let import = HealthImport {
            gap: HealthBackfill {
                provider: &Down,
                publisher: &Down,
                store: &Down,
                source_id: &source_id,
            },
            yearly: HealthBackfill {
                provider: &Down,
                publisher: &Down,
                store: &Down,
                source_id: &source_id,
            },
            activities: ActivityImport {
                source: &Down,
                archive: &Down,
                store: &Down,
            },
            archive_days: ArchiveDays {
                source: &Down,
                archive: &Down,
                store: &Down,
            },
        };
        pull_once(&Down, &Down, &import, &source_id, jobs, started, || ends).await
    }

    fn channel() -> watch::Sender<ImportStatus> {
        watch::channel(ImportStatus::default()).0
    }

    fn retry_at(jobs: &watch::Sender<ImportStatus>) -> Option<Timestamp> {
        jobs.borrow().jobs[&ImportJob::DailyPull].retry_at
    }

    #[tokio::test]
    async fn a_failed_pull_records_only_the_daily_pull_and_leaves_the_others_alone() {
        let jobs = channel();
        let earlier = ImportAttempt {
            started: at("2026-01-14T09:00:00Z"),
            ended: None,
        };
        let earlier_deadline = at("2026-01-15T09:00:00Z");
        jobs.send_modify(|status| status.record(ImportJob::Workouts, earlier, earlier_deadline));
        let before = jobs.borrow().jobs[&ImportJob::Workouts].clone();
        let started = at("2026-01-15T09:00:00Z");
        let next = pull(&jobs, started, after(started, 1)).await;
        let status = jobs.borrow();
        assert_eq!(status.next_pull, Some(next));
        assert_eq!(status.jobs.len(), 2);
        assert_eq!(status.jobs[&ImportJob::Workouts], before);
        let daily = &status.jobs[&ImportJob::DailyPull];
        assert_eq!(daily.attempts.len(), 1);
        assert_eq!(daily.attempts[0].started, started);
        let end = daily.attempts[0].ended.expect("ended");
        assert_eq!(
            end.outcome,
            AttemptOutcome::Failed(ImportFailure::BadAnswer)
        );
        assert_eq!(daily.retry_at, Some(next));
    }

    #[tokio::test]
    async fn the_give_up_waits_for_the_next_ten_oclock_on_either_side_of_it() {
        let jobs = channel();
        let before_ten = at("2026-01-15T07:00:00Z");
        pull(&jobs, before_ten, after(before_ten, 1)).await;
        assert_eq!(retry_at(&jobs), Some(at("2026-01-15T09:00:00Z")));
        let after_ten = at("2026-01-15T10:00:00Z");
        pull(&jobs, after_ten, after(after_ten, 1)).await;
        assert_eq!(retry_at(&jobs), Some(at("2026-01-16T09:00:00Z")));
    }

    #[tokio::test]
    async fn a_startup_pull_and_a_restart_pull_failing_before_ten_share_one_retry_time() {
        let jobs = channel();
        let startup = at("2026-01-15T07:00:00Z");
        pull(&jobs, startup, after(startup, 1)).await;
        let first = retry_at(&jobs);
        let restart = at("2026-01-15T08:00:00Z");
        pull(&jobs, restart, after(restart, 1)).await;
        assert_eq!(retry_at(&jobs), first);
        assert_eq!(first, Some(at("2026-01-15T09:00:00Z")));
    }

    #[tokio::test]
    async fn a_pull_that_overruns_its_deadline_sets_no_retry_time() {
        let jobs = channel();
        let started = at("2026-01-15T07:00:00Z");
        pull(&jobs, started, at("2026-01-15T09:00:01Z")).await;
        let status = jobs.borrow();
        let daily = &status.jobs[&ImportJob::DailyPull];
        assert!(daily.attempts[0].ended.is_some());
        assert_eq!(daily.retry_at, None);
    }
}
