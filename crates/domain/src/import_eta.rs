use serde::{Deserialize, Serialize};

use crate::{ImportJob, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportEta {
    /// The active job's remaining work fits before the next pull.
    At {
        at: Timestamp,
    },
    /// The active job needs this many more daily windows.
    ImportDays {
        days: u32,
    },
    /// The recorded chain state shows `job` must finish first.
    WaitsFor {
        job: ImportJob,
    },
    Unknown,
}
