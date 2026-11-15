use serde::{Deserialize, Serialize};

use crate::Timestamp;

/// The offline alert last delivered for a sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "OfflineAlertEntry")]
pub struct OfflineAlert {
    pub last_seen: Timestamp,
    /// When the first notification of this episode was delivered.
    pub since: Timestamp,
    pub notified_at: Timestamp,
}

#[derive(Deserialize)]
struct OfflineAlertEntry {
    last_seen: Timestamp,
    since: Option<Timestamp>,
    notified_at: Timestamp,
}

impl From<OfflineAlertEntry> for OfflineAlert {
    // Older files lack `since`; the last notification is the closest known.
    fn from(entry: OfflineAlertEntry) -> Self {
        Self {
            last_seen: entry.last_seen,
            since: entry.since.unwrap_or(entry.notified_at),
            notified_at: entry.notified_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Timestamp;

    use super::OfflineAlert;

    fn alert() -> OfflineAlert {
        OfflineAlert {
            last_seen: Timestamp::from_micros(10),
            since: Timestamp::from_micros(20),
            notified_at: Timestamp::from_micros(30),
        }
    }

    #[test]
    fn a_missing_since_is_the_last_notification() {
        let alert: OfflineAlert =
            serde_json::from_str(r#"{"last_seen":10,"notified_at":30}"#).expect("parses");
        assert_eq!(alert.since, alert.notified_at);
    }

    #[test]
    fn round_trips() {
        let json = serde_json::to_string(&alert()).expect("serializes");
        assert_eq!(json, r#"{"last_seen":10,"since":20,"notified_at":30}"#);
        assert_eq!(
            serde_json::from_str::<OfflineAlert>(&json).expect("parses"),
            alert()
        );
    }
}
