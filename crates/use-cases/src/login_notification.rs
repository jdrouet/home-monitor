use domain::LoginEvent;

use crate::Notification;

const LOGIN_COMMANDS: &str = "Health data is no longer collected. On the Pi, run:\n\
docker compose stop health-collector\n\
docker compose run --rm health-collector login\n\
docker compose up -d --force-recreate health-collector";

pub(crate) fn login_notification(event: LoginEvent) -> Notification {
    let (title, message) = match event {
        LoginEvent::Expired => ("Garmin login expired", LOGIN_COMMANDS),
        LoginEvent::StillExpired => ("Garmin login still expired", LOGIN_COMMANDS),
        LoginEvent::Back => (
            "Garmin login works again",
            "Health data is collected again; missed days of the last 30 are being filled.",
        ),
    };
    Notification {
        title: title.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use domain::LoginEvent;

    use super::login_notification;

    #[test]
    fn expired_and_reminder_carry_the_login_commands() {
        for (event, title) in [
            (LoginEvent::Expired, "Garmin login expired"),
            (LoginEvent::StillExpired, "Garmin login still expired"),
        ] {
            let notification = login_notification(event);
            assert_eq!(notification.title, title);
            assert!(
                notification
                    .message
                    .ends_with("docker compose run --rm health-collector login\ndocker compose up -d --force-recreate health-collector")
            );
            assert!(
                notification
                    .message
                    .contains("docker compose stop health-collector\n")
            );
        }
    }

    #[test]
    fn back_says_the_gap_is_being_filled() {
        let notification = login_notification(LoginEvent::Back);
        assert_eq!(notification.title, "Garmin login works again");
        assert!(notification.message.contains("missed days of the last 30"));
    }
}
