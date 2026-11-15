use serde::Deserialize;

/// The figures read from the daily sleep data; a missing or null level at any
/// depth is `None`.
#[derive(Debug, Deserialize)]
pub struct DailySleep {
    #[serde(rename = "dailySleepDTO")]
    sleep: Option<Sleep>,
}

#[derive(Debug, Deserialize)]
struct Sleep {
    #[serde(rename = "sleepTimeSeconds")]
    seconds: Option<u32>,
    #[serde(rename = "sleepScores")]
    scores: Option<Scores>,
}

#[derive(Debug, Deserialize)]
struct Scores {
    overall: Option<Overall>,
}

#[derive(Debug, Deserialize)]
struct Overall {
    value: Option<u32>,
}

impl DailySleep {
    pub fn seconds(&self) -> Option<u32> {
        self.sleep.as_ref()?.seconds
    }

    pub fn score(&self) -> Option<u32> {
        self.sleep
            .as_ref()?
            .scores
            .as_ref()?
            .overall
            .as_ref()?
            .value
    }
}

#[cfg(test)]
mod tests {
    use super::DailySleep;

    fn parse(text: &str) -> DailySleep {
        serde_json::from_str(text).expect("valid sleep data")
    }

    #[test]
    fn the_fixture_gives_duration_and_score() {
        let sleep = parse(include_str!("../fixtures/daily_sleep.json"));
        assert_eq!(sleep.seconds(), Some(27_120));
        assert_eq!(sleep.score(), Some(81));
    }

    #[test]
    fn the_empty_fixture_gives_nothing() {
        let sleep = parse(include_str!("../fixtures/daily_sleep_empty.json"));
        assert_eq!(sleep.seconds(), None);
        assert_eq!(sleep.score(), None);
    }

    #[test]
    fn a_missing_dto_or_missing_levels_give_nothing() {
        for text in [
            r"{}",
            r#"{"dailySleepDTO":null}"#,
            r#"{"dailySleepDTO":{"sleepScores":{}}}"#,
            r#"{"dailySleepDTO":{"sleepScores":{"overall":null}}}"#,
        ] {
            let sleep = parse(text);
            assert_eq!(sleep.seconds(), None);
            assert_eq!(sleep.score(), None);
        }
    }
}
