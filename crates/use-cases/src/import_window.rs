use std::time::Duration;

use domain::Timestamp;
use tokio::time::sleep;
use tracing::warn;

use crate::{HealthProvider, RunEnd};

/// Wait before every Garmin step, a session open or one unit of work, to stay
/// far below the rate that gets clients blocked.
pub(crate) const STEP_SPACING: Duration = Duration::from_secs(60);
/// Time left before the deadline that a step must not eat into.
pub(crate) const MARGIN: Duration = Duration::from_secs(5 * 60);
/// A session is replaced after this long, well within the token's hour.
pub(crate) const SESSION_MAX_AGE: Duration = Duration::from_secs(30 * 60);

pub(crate) struct Opened<T> {
    pub(crate) session: T,
    pub(crate) at: Timestamp,
}

/// Whether work may go on inside an opened session.
#[cfg_attr(not(test), expect(dead_code, reason = "used by the archive runners"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fit {
    Yes,
    /// Less than `MARGIN` is left before the deadline.
    Deadline,
    /// The session is due for replacement.
    SessionAge,
}

/// Waits `STEP_SPACING` if a step fits before `until`, checking again after
/// the wait so a late wake starts nothing.
pub(crate) async fn turn(until: Timestamp, clock: &impl Fn() -> Timestamp) -> bool {
    if until.duration_since(clock()) < STEP_SPACING + MARGIN {
        return false;
    }
    sleep(STEP_SPACING).await;
    until.duration_since(clock()) >= MARGIN
}

/// `current`, else a new session after its own wait; the end when there is no
/// time left or Garmin refuses the session.
pub(crate) async fn open_session<H: HealthProvider>(
    provider: &H,
    current: Option<Opened<H::Session>>,
    until: Timestamp,
    clock: &impl Fn() -> Timestamp,
) -> Result<Opened<H::Session>, RunEnd> {
    if let Some(current) = current {
        return Ok(current);
    }
    if !turn(until, clock).await {
        return Err(RunEnd::OutOfTime);
    }
    match provider.open().await {
        Ok(session) => Ok(Opened {
            session,
            at: clock(),
        }),
        Err(error) => {
            warn!(%error, "import session failed, resuming after the next pull");
            Err(RunEnd::Stopped)
        }
    }
}

/// Whether a request may start now in the session opened at `opened_at`.
#[cfg_attr(not(test), expect(dead_code, reason = "used by the archive runners"))]
pub(crate) fn fits(until: Timestamp, opened_at: Timestamp, clock: &impl Fn() -> Timestamp) -> Fit {
    let now = clock();
    if until.duration_since(now) < MARGIN {
        Fit::Deadline
    } else if now.duration_since(opened_at) >= SESSION_MAX_AGE {
        Fit::SessionAge
    } else {
        Fit::Yes
    }
}

#[cfg(test)]
mod tests {
    use domain::Timestamp;

    use super::{Fit, MARGIN, SESSION_MAX_AGE, fits};

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_micros(seconds * 1_000_000)
    }

    fn margin() -> i64 {
        i64::try_from(MARGIN.as_secs()).expect("small")
    }

    fn max_age() -> i64 {
        i64::try_from(SESSION_MAX_AGE.as_secs()).expect("small")
    }

    #[test]
    fn a_request_fits_with_time_and_a_young_session() {
        let until = at(100_000);
        assert_eq!(fits(until, at(0), &|| at(10)), Fit::Yes);
        assert_eq!(fits(until, at(0), &|| at(max_age() - 1)), Fit::Yes);
    }

    #[test]
    fn nothing_starts_inside_the_margin() {
        let until = at(10_000);
        assert_eq!(fits(until, at(9_000), &|| at(10_000 - margin())), Fit::Yes);
        assert_eq!(
            fits(until, at(9_000), &|| at(10_000 - margin() + 1)),
            Fit::Deadline
        );
    }

    #[test]
    fn an_old_session_is_due_for_replacement() {
        let until = at(100_000);
        assert_eq!(fits(until, at(0), &|| at(max_age())), Fit::SessionAge);
    }

    #[test]
    fn the_deadline_wins_over_the_session_age() {
        let until = at(max_age() + 10);
        assert_eq!(fits(until, at(0), &|| at(max_age())), Fit::Deadline);
    }
}
