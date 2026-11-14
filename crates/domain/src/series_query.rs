use std::num::NonZeroU32;

use serde::Deserialize;

use crate::{Metric, SourceId, Timestamp};

/// Selects the readings of one metric over a time range. `from` is inclusive
/// and `to` exclusive. With `bucket_secs`, readings are averaged per source
/// into epoch-aligned buckets of that width.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SeriesQuery {
    pub metric: Metric,
    pub source_id: Option<SourceId>,
    pub from: Timestamp,
    pub to: Timestamp,
    pub bucket_secs: Option<NonZeroU32>,
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::SeriesQuery;
    use crate::{Metric, Timestamp};

    #[test]
    fn deserializes_all_fields() {
        let query: SeriesQuery = serde_json::from_str(
            r#"{"metric":"humidity","source_id":"AA","from":1,"to":9,"bucket_secs":60}"#,
        )
        .expect("valid query");
        assert_eq!(query.metric, Metric::Humidity);
        assert_eq!(query.source_id.as_ref().map(|id| id.as_str()), Some("AA"));
        assert_eq!(query.from, Timestamp::from_micros(1));
        assert_eq!(query.to, Timestamp::from_micros(9));
        assert_eq!(query.bucket_secs, NonZeroU32::new(60));
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let query: SeriesQuery =
            serde_json::from_str(r#"{"metric":"battery","from":0,"to":1}"#).expect("valid query");
        assert_eq!((query.source_id, query.bucket_secs), (None, None));
    }

    #[test]
    fn zero_bucket_is_rejected() {
        let outcome = serde_json::from_str::<SeriesQuery>(
            r#"{"metric":"battery","from":0,"to":1,"bucket_secs":0}"#,
        );
        assert!(outcome.is_err());
    }
}
