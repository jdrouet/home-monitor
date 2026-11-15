use crate::ActivityCursor;

/// Keeps the activity import position across restarts.
pub trait ActivityCursorStore {
    type Error: std::error::Error + Send + Sync + 'static;

    /// `None` when the import has not started.
    fn load(&self) -> Result<Option<ActivityCursor>, Self::Error>;

    fn save(&self, cursor: &ActivityCursor) -> Result<(), Self::Error>;
}
