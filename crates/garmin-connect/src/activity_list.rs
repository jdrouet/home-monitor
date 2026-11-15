use serde::Deserialize;
use use_cases::{ActivityId, ActivityRef};

use crate::GarminError;

pub(crate) const ENDPOINT: &str = "activity list";
/// Length of the `YYYY-MM-DD` start of a local start time.
const DATE_LEN: usize = 10;

/// The activity list: a bare array, or an object holding one.
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ActivityList {
    Bare(Vec<Item>),
    Wrapped {
        #[serde(rename = "activityList")]
        items: Vec<Item>,
    },
}

#[derive(Deserialize)]
pub(crate) struct Item {
    #[serde(rename = "activityId")]
    id: u64,
    #[serde(rename = "startTimeLocal")]
    start: String,
}

impl ActivityList {
    /// The activities in list order, or an error when one has no usable day.
    pub(crate) fn into_refs(self) -> Result<Vec<ActivityRef>, GarminError> {
        let (Self::Bare(items) | Self::Wrapped { items }) = self;
        items
            .into_iter()
            .map(|item| {
                let date = item.start.get(..DATE_LEN)?.parse().ok()?;
                Some(ActivityRef {
                    id: ActivityId(item.id),
                    date,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(GarminError::InvalidResponse { endpoint: ENDPOINT })
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use use_cases::{ActivityId, ActivityRef};

    use crate::GarminError;

    use super::ActivityList;

    const FIXTURE: &str = include_str!("../fixtures/activity_list.json");

    fn refs(json: &str) -> Result<Vec<ActivityRef>, GarminError> {
        serde_json::from_str::<ActivityList>(json)
            .map_err(|_| GarminError::Malformed)?
            .into_refs()
    }

    fn expected() -> Vec<ActivityRef> {
        vec![
            ActivityRef {
                id: ActivityId(22_222),
                date: NaiveDate::from_ymd_opt(2026, 7, 14).expect("date"),
            },
            ActivityRef {
                id: ActivityId(11_111),
                date: NaiveDate::from_ymd_opt(2026, 7, 12).expect("date"),
            },
        ]
    }

    #[test]
    fn a_bare_array_is_read() {
        assert_eq!(refs(FIXTURE).expect("refs"), expected());
    }

    #[test]
    fn a_wrapped_list_is_read() {
        let wrapped = format!(r#"{{"activityList":{FIXTURE}}}"#);
        assert_eq!(refs(&wrapped).expect("refs"), expected());
    }

    #[test]
    fn an_empty_list_is_empty() {
        assert!(refs("[]").expect("refs").is_empty());
    }

    #[test]
    fn a_missing_id_does_not_parse() {
        let json = r#"[{"startTimeLocal":"2026-07-14 08:00:00"}]"#;
        assert!(refs(json).is_err());
    }

    #[test]
    fn a_bad_or_short_start_is_an_invalid_response() {
        for start in ["yesterday at 8", "2026-07", "2026-13-40 08:00:00"] {
            let json = format!(r#"[{{"activityId":1,"startTimeLocal":"{start}"}}]"#);
            assert!(
                matches!(
                    refs(&json),
                    Err(GarminError::InvalidResponse {
                        endpoint: "activity list"
                    })
                ),
                "{start}"
            );
        }
    }
}
