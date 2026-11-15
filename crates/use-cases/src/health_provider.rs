use std::future::Future;

use chrono::NaiveDate;

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
}
