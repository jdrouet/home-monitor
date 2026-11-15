use crate::BackfillCursor;

/// Keeps the health backfill position across restarts.
pub trait BackfillStore {
    type Error: std::error::Error + Send + Sync + 'static;

    /// `None` when the backfill has not started.
    fn load(&self) -> Result<Option<BackfillCursor>, Self::Error>;

    fn save(&self, cursor: &BackfillCursor) -> Result<(), Self::Error>;
}
