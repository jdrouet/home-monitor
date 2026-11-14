#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("payload is encrypted")]
    Encrypted,
    #[error("unsupported BTHome version {0}")]
    UnsupportedVersion(u8),
    #[error("unsupported object id {0:#04x}")]
    UnsupportedObject(u8),
    #[error("payload ends inside an object")]
    Truncated,
}
