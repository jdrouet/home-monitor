use std::cmp::Reverse;

use domain::{AlertKind, AlertState, SourceId, Thermometer, Timestamp};

/// An alert delivered and not yet resolved. `celsius` is the temperature at
/// the last notification, set for temperature alerts only.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAlertLine {
    pub subject: String,
    pub kind: AlertKind,
    pub celsius: Option<f64>,
    pub since: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAlertLine {
    pub subject: String,
    pub kind: AlertKind,
    pub started: Timestamp,
    pub ended: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertSummary {
    /// Oldest onset first.
    pub active: Vec<ActiveAlertLine>,
    /// Most recently ended first.
    pub resolved: Vec<ResolvedAlertLine>,
}

/// Names each alert after its room; a source that is not configured keeps
/// its source id.
pub fn build_alert_summary(thermometers: &[Thermometer], state: &AlertState) -> AlertSummary {
    let subject = |source_id: &SourceId| {
        thermometers
            .iter()
            .find(|thermometer| SourceId::from(thermometer.mac) == *source_id)
            .map_or_else(
                || source_id.as_str().to_owned(),
                |thermometer| thermometer.room.to_string(),
            )
    };
    let offline = state
        .offline
        .iter()
        .map(|(source_id, alert)| ActiveAlertLine {
            subject: subject(source_id),
            kind: AlertKind::Offline,
            celsius: None,
            since: alert.since,
        });
    let temperature = state
        .temperature
        .iter()
        .map(|(source_id, alert)| ActiveAlertLine {
            subject: subject(source_id),
            kind: alert.deviation.into(),
            celsius: Some(alert.celsius),
            since: alert.since,
        });
    let mut active: Vec<ActiveAlertLine> = offline.chain(temperature).collect();
    active.sort_by_key(|line| line.since);
    let mut resolved: Vec<ResolvedAlertLine> = state
        .history
        .iter()
        .map(|alert| ResolvedAlertLine {
            subject: subject(&alert.source_id),
            kind: alert.kind,
            started: alert.started,
            ended: alert.ended,
        })
        .collect();
    resolved.sort_by_key(|line| Reverse(line.ended));
    AlertSummary { active, resolved }
}

#[cfg(test)]
mod tests {
    use domain::{
        AlertKind, AlertState, Deviation, MacAddress, OfflineAlert, ResolvedAlert, RoomName,
        SourceId, TemperatureAlert, Thermometer, Timestamp,
    };

    use super::build_alert_summary;

    const MAC_BEDROOM: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 1]);
    const MAC_OFFICE: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 2]);
    const MAC_STRANGER: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9]);

    fn at(micros: i64) -> Timestamp {
        Timestamp::from_micros(micros)
    }

    fn thermometers() -> Vec<Thermometer> {
        [(MAC_BEDROOM, "bedroom"), (MAC_OFFICE, "office")]
            .into_iter()
            .map(|(mac, room)| Thermometer {
                mac,
                room: RoomName::try_from(room.to_owned()).expect("valid room"),
                temperature_range: None,
            })
            .collect()
    }

    fn offline(since: i64) -> OfflineAlert {
        OfflineAlert {
            last_seen: at(0),
            since: at(since),
            notified_at: at(since),
        }
    }

    fn hot(celsius: f64, since: i64) -> TemperatureAlert {
        TemperatureAlert {
            deviation: Deviation::TooHot,
            celsius,
            since: at(since),
            notified_at: at(since),
        }
    }

    fn resolved(mac: MacAddress, kind: AlertKind, started: i64, ended: i64) -> ResolvedAlert {
        ResolvedAlert {
            source_id: mac.into(),
            kind,
            started: at(started),
            ended: at(ended),
        }
    }

    #[test]
    fn nothing_to_show_is_empty() {
        let summary = build_alert_summary(&thermometers(), &AlertState::default());
        assert!(summary.active.is_empty());
        assert!(summary.resolved.is_empty());
    }

    #[test]
    fn alerts_are_named_after_their_room() {
        let mut state = AlertState::default();
        state.offline.insert(MAC_BEDROOM.into(), offline(10));
        state
            .history
            .push(resolved(MAC_OFFICE, AlertKind::TooCold, 1, 2));
        let summary = build_alert_summary(&thermometers(), &state);
        assert_eq!(summary.active[0].subject, "bedroom");
        assert_eq!(summary.resolved[0].subject, "office");
    }

    #[test]
    fn unknown_sources_are_named_by_id() {
        let mut state = AlertState::default();
        state.offline.insert(MAC_STRANGER.into(), offline(10));
        state
            .history
            .push(resolved(MAC_STRANGER, AlertKind::Offline, 1, 2));
        let summary = build_alert_summary(&thermometers(), &state);
        let id = SourceId::from(MAC_STRANGER).as_str().to_owned();
        assert_eq!(summary.active[0].subject, id);
        assert_eq!(summary.resolved[0].subject, id);
    }

    #[test]
    fn celsius_is_set_for_temperature_alerts_only() {
        let mut state = AlertState::default();
        state.offline.insert(MAC_BEDROOM.into(), offline(10));
        state.temperature.insert(MAC_OFFICE.into(), hot(21.3, 20));
        let summary = build_alert_summary(&thermometers(), &state);
        assert_eq!(summary.active[0].kind, AlertKind::Offline);
        assert_eq!(summary.active[0].celsius, None);
        assert_eq!(summary.active[1].kind, AlertKind::TooHot);
        assert_eq!(summary.active[1].celsius, Some(21.3));
    }

    #[test]
    fn active_alerts_are_oldest_first() {
        let mut state = AlertState::default();
        state.offline.insert(MAC_BEDROOM.into(), offline(30));
        state.temperature.insert(MAC_OFFICE.into(), hot(21.3, 20));
        state.offline.insert(MAC_OFFICE.into(), offline(10));
        let summary = build_alert_summary(&thermometers(), &state);
        let since: Vec<_> = summary.active.iter().map(|line| line.since).collect();
        assert_eq!(since, [at(10), at(20), at(30)]);
    }

    #[test]
    fn resolved_alerts_are_newest_first() {
        let mut state = AlertState::default();
        state
            .history
            .push(resolved(MAC_BEDROOM, AlertKind::Offline, 1, 20));
        state
            .history
            .push(resolved(MAC_BEDROOM, AlertKind::TooHot, 5, 40));
        state
            .history
            .push(resolved(MAC_OFFICE, AlertKind::TooCold, 3, 30));
        let summary = build_alert_summary(&thermometers(), &state);
        let ended: Vec<_> = summary.resolved.iter().map(|line| line.ended).collect();
        assert_eq!(ended, [at(40), at(30), at(20)]);
        assert_eq!(summary.resolved[0].started, at(5));
        assert_eq!(summary.resolved[0].kind, AlertKind::TooHot);
    }
}
