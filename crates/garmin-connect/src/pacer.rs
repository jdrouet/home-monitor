use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::ops::RangeInclusive;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::{Instant, sleep_until};

/// Spaces requests by a random gap. One pacer is shared process-wide so the
/// login, the refresh and every data call together keep to the pace.
pub struct Pacer {
    gap: RangeInclusive<Duration>,
    last: Mutex<Option<Instant>>,
}

impl Pacer {
    pub const fn new(gap: RangeInclusive<Duration>) -> Self {
        Self {
            gap,
            last: Mutex::new(None),
        }
    }

    /// Returns when a request may be sent, and counts it as sent. One gap is
    /// drawn per call; the lock is re-checked after every sleep so concurrent
    /// callers queue behind each other.
    pub async fn wait(&self) {
        let gap = jitter(*self.gap.start(), *self.gap.end(), sample());
        while let Err(due) = self.claim(Instant::now(), gap) {
            sleep_until(due).await;
        }
    }

    /// Records a send at `now`, or returns when the gap after the previous
    /// send ends.
    pub(crate) fn claim(&self, now: Instant, gap: Duration) -> Result<(), Instant> {
        let mut last = self.last.lock().expect("pacer lock is never poisoned");
        match *last {
            Some(previous) if previous + gap > now => Err(previous + gap),
            _ => {
                *last = Some(now);
                Ok(())
            }
        }
    }
}

/// A duration in `min..=max`, picked by `sample`.
pub(crate) fn jitter(min: Duration, max: Duration, sample: u64) -> Duration {
    let span = (max - min).as_nanos() + 1;
    let offset = u64::try_from(u128::from(sample) % span).expect("offset is below the span");
    min + Duration::from_nanos(offset)
}

fn sample() -> u64 {
    RandomState::new().build_hasher().finish()
}

#[cfg(test)]
pub(crate) static NO_GAP: Pacer = Pacer::new(Duration::ZERO..=Duration::ZERO);

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::{Pacer, jitter};

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn jitter_stays_within_the_bounds() {
        let (min, max) = (3 * SECOND, 6 * SECOND);
        assert_eq!(jitter(min, max, 0), min);
        assert_eq!(jitter(min, max, 3_000_000_000), max);
        assert_eq!(jitter(min, max, 3_000_000_001), min);
        for sample in [1, 12_345, u64::MAX] {
            let gap = jitter(min, max, sample);
            assert!((min..=max).contains(&gap), "{gap:?}");
        }
        assert_eq!(jitter(SECOND, SECOND, u64::MAX), SECOND);
    }

    #[tokio::test(start_paused = true)]
    async fn claim_refuses_before_the_gap_and_accepts_at_it() {
        let pacer = Pacer::new(SECOND..=SECOND);
        let start = Instant::now();
        assert_eq!(pacer.claim(start, 3 * SECOND), Ok(()));
        assert_eq!(
            pacer.claim(start + 2 * SECOND, 3 * SECOND),
            Err(start + 3 * SECOND)
        );
        assert_eq!(pacer.claim(start + 3 * SECOND, 3 * SECOND), Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_waits_are_spaced_by_the_gap() {
        let pacer = Pacer::new(3 * SECOND..=3 * SECOND);
        let start = Instant::now();
        pacer.wait().await;
        assert_eq!(start.elapsed(), Duration::ZERO);
        let waited = async {
            pacer.wait().await;
            start.elapsed()
        };
        let (first, second) = tokio::join!(waited, async {
            pacer.wait().await;
            start.elapsed()
        });
        assert_eq!(first.min(second), 3 * SECOND);
        assert_eq!(first.max(second), 6 * SECOND);
    }
}
