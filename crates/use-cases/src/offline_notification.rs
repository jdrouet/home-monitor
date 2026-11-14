use std::time::Duration;

use domain::{OfflineEvent, RoomName};

use crate::Notification;

pub(crate) fn offline_notification(room: &RoomName, event: OfflineEvent) -> Notification {
    let (title, message) = match event {
        OfflineEvent::Offline { silent_for } => (
            format!("{room} thermometer offline"),
            silent_message(silent_for),
        ),
        OfflineEvent::StillOffline { silent_for } => (
            format!("{room} thermometer still offline"),
            silent_message(silent_for),
        ),
        OfflineEvent::Back => (
            format!("{room} thermometer is back"),
            "Reporting again.".to_owned(),
        ),
    };
    Notification { title, message }
}

fn silent_message(silent_for: Duration) -> String {
    format!("No reading for {} h.", silent_for.as_secs() / 3600)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn silent_message_rounds_down_to_whole_hours() {
        assert_eq!(
            super::silent_message(Duration::from_secs(2 * 3600 + 3599)),
            "No reading for 2 h."
        );
    }
}
