use domain::{Metric, RoomName, Thermometer};
use use_cases::{HISTORY_METRICS, HistorySelection, Period};

fn value<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// Reads the history form fields. Invalid values fall back to temperature and
/// 24 h, unknown rooms are ignored, and with nothing selected every room and
/// the outside line are shown.
pub fn parse(pairs: &[(String, String)], thermometers: &[Thermometer]) -> HistorySelection {
    let metric = HISTORY_METRICS
        .into_iter()
        .find(|metric| Some(metric.as_str()) == value(pairs, "metric"))
        .unwrap_or(Metric::Temperature);
    let period = Period::ALL
        .into_iter()
        .find(|period| Some(period.as_str()) == value(pairs, "period"))
        .unwrap_or(Period::Day);
    let mut rooms: Vec<RoomName> = thermometers
        .iter()
        .map(|thermometer| &thermometer.room)
        .filter(|room| {
            pairs
                .iter()
                .any(|(name, value)| name == "room" && value == room.as_str())
        })
        .cloned()
        .collect();
    let mut include_outside = value(pairs, "outside") == Some("on");
    if rooms.is_empty() && !include_outside {
        rooms = thermometers.iter().map(|t| t.room.clone()).collect();
        include_outside = true;
    }
    HistorySelection {
        metric,
        period,
        rooms,
        include_outside,
    }
}

#[cfg(test)]
mod tests {
    use domain::{MacAddress, Metric, RoomName, Thermometer};
    use use_cases::Period;

    use super::parse;

    fn thermometers() -> Vec<Thermometer> {
        [("bedroom", 1), ("office", 2)]
            .into_iter()
            .map(|(room, last)| Thermometer {
                mac: MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, last]),
                room: RoomName::try_from(room.to_owned()).expect("valid room"),
                temperature_range: None,
            })
            .collect()
    }

    fn pairs(raw: &[(&str, &str)]) -> Vec<(String, String)> {
        raw.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn room_names(rooms: &[RoomName]) -> Vec<&str> {
        rooms.iter().map(RoomName::as_str).collect()
    }

    #[test]
    fn no_parameters_select_everything() {
        let selection = parse(&[], &thermometers());
        assert_eq!(selection.metric, Metric::Temperature);
        assert_eq!(selection.period, Period::Day);
        assert_eq!(room_names(&selection.rooms), ["bedroom", "office"]);
        assert!(selection.include_outside);
    }

    #[test]
    fn valid_parameters_are_applied() {
        let raw = [("metric", "humidity"), ("period", "1y"), ("room", "office")];
        let selection = parse(&pairs(&raw), &thermometers());
        assert_eq!(selection.metric, Metric::Humidity);
        assert_eq!(selection.period, Period::Year);
        assert_eq!(room_names(&selection.rooms), ["office"]);
        assert!(!selection.include_outside);
    }

    #[test]
    fn only_unknown_rooms_select_everything() {
        let selection = parse(&pairs(&[("room", "garage")]), &thermometers());
        assert_eq!(room_names(&selection.rooms), ["bedroom", "office"]);
        assert!(selection.include_outside);
    }

    #[test]
    fn outside_alone_is_kept() {
        let selection = parse(&pairs(&[("outside", "on")]), &thermometers());
        assert!(selection.rooms.is_empty());
        assert!(selection.include_outside);
    }

    #[test]
    fn bogus_values_fall_back_to_the_defaults() {
        let raw = [
            ("metric", "battery"),
            ("period", "forever"),
            ("room", "office"),
        ];
        let selection = parse(&pairs(&raw), &thermometers());
        assert_eq!(selection.metric, Metric::Temperature);
        assert_eq!(selection.period, Period::Day);
    }
}
