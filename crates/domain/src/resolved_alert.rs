use serde::{Deserialize, Serialize};

use crate::{AlertKind, SourceId, Timestamp};

/// An alert that ended: first notified at `started`, resolved at `ended`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAlert {
    pub source_id: SourceId,
    pub kind: AlertKind,
    pub started: Timestamp,
    pub ended: Timestamp,
}
