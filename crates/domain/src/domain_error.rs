/// Failure to build a domain value from untrusted input.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid MAC address: {0:?}")]
    InvalidMacAddress(String),
    #[error("room name must not be empty")]
    EmptyRoomName,
    #[error("unknown source kind: {0:?}")]
    UnknownSourceKind(String),
    #[error("min_temperature and max_temperature must be set together")]
    IncompleteTemperatureRange,
    #[error("min_temperature must be below max_temperature, and both finite")]
    InvalidTemperatureRange,
}
