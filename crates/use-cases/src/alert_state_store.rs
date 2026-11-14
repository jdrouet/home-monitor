use domain::AlertState;

/// Keeps the delivered alert state across restarts.
pub trait AlertStateStore {
    type Error: std::error::Error + Send + Sync + 'static;

    fn load(&self) -> Result<AlertState, Self::Error>;

    fn save(&self, state: &AlertState) -> Result<(), Self::Error>;
}
