use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Microseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_micros(micros: i64) -> Self {
        Self(micros)
    }

    pub const fn as_micros(self) -> i64 {
        self.0
    }

    pub fn now() -> Self {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch");
        let micros = i64::try_from(since_epoch.as_micros()).expect("timestamp overflows i64");
        Self(micros)
    }

    /// Time elapsed since `earlier`; zero when `earlier` is not earlier.
    pub fn duration_since(self, earlier: Self) -> Duration {
        let micros = self.0.saturating_sub(earlier.0);
        Duration::from_micros(u64::try_from(micros).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Timestamp;

    #[test]
    fn duration_since_measures_the_gap() {
        let later = Timestamp::from_micros(3_000_000);
        let earlier = Timestamp::from_micros(1_000_000);
        assert_eq!(later.duration_since(earlier), Duration::from_secs(2));
    }

    #[test]
    fn duration_since_saturates_at_zero() {
        let later = Timestamp::from_micros(3_000_000);
        let earlier = Timestamp::from_micros(1_000_000);
        assert_eq!(earlier.duration_since(later), Duration::ZERO);
        assert_eq!(
            Timestamp::from_micros(i64::MIN).duration_since(Timestamp::from_micros(i64::MAX)),
            Duration::ZERO
        );
    }

    #[test]
    fn now_is_after_2020() {
        assert!(Timestamp::now().as_micros() > 1_577_836_800_000_000);
    }
}
