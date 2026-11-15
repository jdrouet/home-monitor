use serde::Deserialize;

/// The figures read from the daily user summary; missing or null is `None`.
#[derive(Debug, Deserialize)]
pub struct DailySummary {
    #[serde(rename = "totalSteps")]
    pub steps: Option<u32>,
    #[serde(rename = "restingHeartRate")]
    pub resting_heart_rate: Option<u32>,
    #[serde(rename = "bodyBatteryHighestValue")]
    pub body_battery_max: Option<u32>,
    #[serde(rename = "bodyBatteryLowestValue")]
    pub body_battery_min: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::DailySummary;

    #[test]
    fn the_fixture_gives_every_figure() {
        let summary: DailySummary =
            serde_json::from_str(include_str!("../fixtures/user_summary.json")).expect("valid");
        assert_eq!(summary.steps, Some(8431));
        assert_eq!(summary.resting_heart_rate, Some(52));
        assert_eq!(summary.body_battery_max, Some(96));
        assert_eq!(summary.body_battery_min, Some(14));
    }

    #[test]
    fn nulls_and_missing_keys_are_none() {
        let summary: DailySummary =
            serde_json::from_str(r#"{"totalSteps":null,"restingHeartRate":null}"#).expect("valid");
        assert_eq!(summary.steps, None);
        assert_eq!(summary.resting_heart_rate, None);
        assert_eq!(summary.body_battery_max, None);
        assert_eq!(summary.body_battery_min, None);
    }
}
