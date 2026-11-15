use crate::ArchiveKey;

/// Keeps raw Garmin documents, one per key, exactly as received.
pub trait RawArchive {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Whether the document or its denied marker is stored.
    fn contains(&self, key: &ArchiveKey) -> Result<bool, Self::Error>;

    /// Stores `bytes`, replacing any earlier document. An empty slice records
    /// that Garmin has nothing for the key.
    fn put(&self, key: &ArchiveKey, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Records that Garmin refused the document, keeping it identifiable and
    /// distinct from a stored one.
    fn put_denied(&self, key: &ArchiveKey) -> Result<(), Self::Error>;
}
