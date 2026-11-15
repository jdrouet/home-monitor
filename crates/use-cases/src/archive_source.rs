use std::future::Future;

use crate::{ActivityRef, DocumentKey, Fetched, HealthProvider};

/// Reads Garmin's original documents through a session.
pub trait ArchiveSource: HealthProvider {
    type Document: Copy + Send + Sync + 'static;

    /// The documents kept for every completed day.
    const DAILY: &'static [Self::Document];
    /// The documents kept for every activity.
    const ACTIVITY: &'static [Self::Document];

    fn file_name(document: Self::Document) -> &'static str;

    fn document(
        &self,
        session: &Self::Session,
        document: Self::Document,
        key: DocumentKey,
    ) -> impl Future<Output = Result<Fetched, Self::Error>> + Send;

    /// `limit` activities from position `start` of the list, newest first.
    fn activities(
        &self,
        session: &Self::Session,
        start: u32,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ActivityRef>, Self::Error>> + Send;
}
