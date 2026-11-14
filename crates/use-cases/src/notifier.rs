use std::future::Future;

use crate::Notification;

pub trait Notifier {
    type Error: std::error::Error + Send + Sync + 'static;

    fn notify(
        &self,
        notification: &Notification,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
