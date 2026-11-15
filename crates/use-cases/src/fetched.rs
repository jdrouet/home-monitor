/// What Garmin answered for one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// The document, as Garmin sent it; empty for a 204.
    Body(Vec<u8>),
    /// Garmin confirmed it has no such document (404).
    Absent,
    /// Garmin refused the request for this document (403).
    Denied,
}
