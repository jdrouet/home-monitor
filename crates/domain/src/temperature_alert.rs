use serde::{Deserialize, Serialize};

use crate::{Deviation, Timestamp};

/// The temperature alert last delivered for a room.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "TemperatureAlertEntry")]
pub struct TemperatureAlert {
    pub deviation: Deviation,
    /// The last temperature known when the notification was delivered.
    pub celsius: f64,
    /// When the first notification of this episode was delivered.
    pub since: Timestamp,
    pub notified_at: Timestamp,
}

#[derive(Deserialize)]
struct TemperatureAlertEntry {
    deviation: Deviation,
    celsius: f64,
    since: Option<Timestamp>,
    notified_at: Timestamp,
}

impl From<TemperatureAlertEntry> for TemperatureAlert {
    // Older files lack `since`; the last notification is the closest known.
    fn from(entry: TemperatureAlertEntry) -> Self {
        Self {
            deviation: entry.deviation,
            celsius: entry.celsius,
            since: entry.since.unwrap_or(entry.notified_at),
            notified_at: entry.notified_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Deviation, Timestamp};

    use super::TemperatureAlert;

    fn alert() -> TemperatureAlert {
        TemperatureAlert {
            deviation: Deviation::TooHot,
            celsius: 21.5,
            since: Timestamp::from_micros(20),
            notified_at: Timestamp::from_micros(30),
        }
    }

    #[test]
    fn a_missing_since_is_the_last_notification() {
        let alert: TemperatureAlert =
            serde_json::from_str(r#"{"deviation":"too_hot","celsius":21.5,"notified_at":30}"#)
                .expect("parses");
        assert_eq!(alert.since, alert.notified_at);
    }

    #[test]
    fn round_trips() {
        let json = serde_json::to_string(&alert()).expect("serializes");
        assert_eq!(
            json,
            r#"{"deviation":"too_hot","celsius":21.5,"since":20,"notified_at":30}"#
        );
        assert_eq!(
            serde_json::from_str::<TemperatureAlert>(&json).expect("parses"),
            alert()
        );
    }
}
