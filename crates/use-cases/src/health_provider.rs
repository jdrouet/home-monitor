use std::future::Future;

use chrono::NaiveDate;
use domain::ImportFailure;

use crate::DailyHealth;

/// Reads daily health figures through a session opened once per pull.
pub trait HealthProvider {
    type Session: Send + Sync;
    type Error: std::error::Error + Send + Sync + 'static;

    fn open(&self) -> impl Future<Output = Result<Self::Session, Self::Error>> + Send;

    fn daily(
        &self,
        session: &Self::Session,
        date: NaiveDate,
    ) -> impl Future<Output = Result<DailyHealth, Self::Error>> + Send;

    /// Whether `error` means the session itself was refused, as opposed to a
    /// failure confined to one day.
    fn is_session_error(error: &Self::Error) -> bool;

    /// Whether `error` means Garmin asks the client to slow down.
    fn is_throttled(error: &Self::Error) -> bool;

    /// The reason shown to the owner when `error` stops a job.
    fn failure(error: &Self::Error) -> ImportFailure {
        if Self::is_session_error(error) {
            ImportFailure::LoginRejected
        } else if Self::is_throttled(error) {
            ImportFailure::Throttled
        } else {
            ImportFailure::BadAnswer
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use domain::ImportFailure;

    use crate::test_archive::TestError;
    use crate::{DailyHealth, HealthProvider};

    struct Provider;

    impl HealthProvider for Provider {
        type Session = ();
        type Error = TestError;

        async fn open(&self) -> Result<(), TestError> {
            Ok(())
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

    #[test]
    fn the_default_reason_follows_the_error_class() {
        for (error, reason) in [
            (TestError::Session, ImportFailure::LoginRejected),
            (TestError::Throttled, ImportFailure::Throttled),
            (TestError::Other, ImportFailure::BadAnswer),
        ] {
            assert_eq!(Provider::failure(&error), reason);
        }
    }
}
