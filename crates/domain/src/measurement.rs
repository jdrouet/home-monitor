use serde::{Deserialize, Serialize};

use crate::{Metric, SourceId, SourceKind, Timestamp};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub timestamp: Timestamp,
    pub source_kind: SourceKind,
    pub source_id: SourceId,
    pub metric: Metric,
    pub value: f64,
}
