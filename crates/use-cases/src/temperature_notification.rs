use domain::{Deviation, RoomName, TemperatureEvent, TemperatureRange};

use crate::Notification;

pub(crate) fn temperature_notification(
    room: &RoomName,
    event: TemperatureEvent,
    range: TemperatureRange,
) -> Notification {
    let (title, celsius) = match event {
        TemperatureEvent::Onset { deviation, celsius } => {
            (format!("{room} {}", direction(deviation)), celsius)
        }
        TemperatureEvent::Reminder { deviation, celsius } => {
            (format!("{room} still {}", direction(deviation)), celsius)
        }
        TemperatureEvent::Resolved { celsius } => {
            (format!("{room} temperature back in range"), celsius)
        }
    };
    let message = format!(
        "{celsius:.1} °C, allowed {} to {} °C.",
        range.min(),
        range.max()
    );
    Notification { title, message }
}

const fn direction(deviation: Deviation) -> &'static str {
    match deviation {
        Deviation::TooCold => "too cold",
        Deviation::TooHot => "too hot",
    }
}

#[cfg(test)]
mod tests {
    use domain::{Deviation, RoomName, TemperatureEvent, TemperatureRange};

    use super::temperature_notification;

    fn titles_and_message(event: TemperatureEvent) -> (String, String) {
        let room = RoomName::try_from("bedroom".to_owned()).expect("valid room");
        let range = TemperatureRange::new(18.0, 20.5).expect("valid range");
        let notification = temperature_notification(&room, event, range);
        (notification.title, notification.message)
    }

    #[test]
    fn onset_names_the_direction() {
        let hot = TemperatureEvent::Onset {
            deviation: Deviation::TooHot,
            celsius: 21.34,
        };
        let cold = TemperatureEvent::Onset {
            deviation: Deviation::TooCold,
            celsius: 16.0,
        };
        assert_eq!(
            titles_and_message(hot),
            (
                "bedroom too hot".to_owned(),
                "21.3 °C, allowed 18 to 20.5 °C.".to_owned()
            )
        );
        assert_eq!(titles_and_message(cold).0, "bedroom too cold");
    }

    #[test]
    fn reminder_says_still() {
        for (deviation, title) in [
            (Deviation::TooHot, "bedroom still too hot"),
            (Deviation::TooCold, "bedroom still too cold"),
        ] {
            let event = TemperatureEvent::Reminder {
                deviation,
                celsius: 19.0,
            };
            assert_eq!(titles_and_message(event).0, title);
        }
    }

    #[test]
    fn resolved_says_back_in_range() {
        let (title, message) = titles_and_message(TemperatureEvent::Resolved { celsius: 19.0 });
        assert_eq!(title, "bedroom temperature back in range");
        assert_eq!(message, "19.0 °C, allowed 18 to 20.5 °C.");
    }
}
