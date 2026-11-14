use std::time::Duration;

use reqwest::{Client, Response, Url};
use use_cases::{Notification, Notifier};

use crate::{NtfyError, NtfyTopic};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Publishes notifications to one topic of a ntfy server.
pub struct NtfyNotifier {
    http: Client,
    publish_url: Url,
}

impl NtfyNotifier {
    pub fn new(url: &str, topic: &NtfyTopic) -> Result<Self, NtfyError> {
        let invalid = || NtfyError::InvalidUrl(url.to_owned());
        let mut publish_url = Url::parse(url).map_err(|_| invalid())?;
        publish_url
            .path_segments_mut()
            .map_err(|()| invalid())?
            .pop_if_empty()
            .push(topic.as_str());
        Ok(Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("http client configuration is valid"),
            publish_url,
        })
    }

    fn request_url(&self, title: &str) -> Url {
        let mut url = self.publish_url.clone();
        url.query_pairs_mut().append_pair("title", title);
        url
    }
}

impl Notifier for NtfyNotifier {
    type Error = NtfyError;

    async fn notify(&self, notification: &Notification) -> Result<(), NtfyError> {
        self.http
            .post(self.request_url(&notification.title))
            .body(notification.message.clone())
            .send()
            .await
            .and_then(Response::error_for_status)
            .map(|_| ())
            .map_err(|error| NtfyError::Send(error.without_url()))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use use_cases::{Notification, Notifier};

    use crate::{NtfyError, NtfyTopic};

    use super::NtfyNotifier;

    const TOPIC: &str = "secrettopic123";

    fn topic() -> NtfyTopic {
        TOPIC.parse().expect("valid topic")
    }

    fn notification() -> Notification {
        Notification {
            title: "bedroom thermometer offline".to_owned(),
            message: "No reading for 1 h.".to_owned(),
        }
    }

    async fn serve_once(status: &'static str) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let url = format!("http://{}", listener.local_addr().expect("has address"));
        let request = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accepts");
            let mut received = Vec::new();
            while !received.ends_with(b"No reading for 1 h.") {
                let mut chunk = [0; 1024];
                let read = stream.read(&mut chunk).await.expect("reads");
                received.extend_from_slice(&chunk[..read]);
            }
            let response =
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            stream.write_all(response.as_bytes()).await.expect("writes");
            String::from_utf8(received).expect("request is text")
        });
        (url, request)
    }

    #[test]
    fn publish_url_appends_the_topic_with_or_without_trailing_slash() {
        for url in ["http://ntfy", "http://ntfy/", "http://ntfy/base/"] {
            let notifier = NtfyNotifier::new(url, &topic()).expect("valid url");
            let expected = format!("{}/{TOPIC}", url.trim_end_matches('/'));
            assert_eq!(notifier.publish_url.as_str(), expected);
        }
    }

    #[test]
    fn invalid_urls_are_rejected() {
        for url in ["not a url", "mailto:someone@example.com"] {
            assert!(matches!(
                NtfyNotifier::new(url, &topic()),
                Err(NtfyError::InvalidUrl(_))
            ));
        }
    }

    #[test]
    fn title_is_percent_encoded() {
        let notifier = NtfyNotifier::new("http://ntfy", &topic()).expect("valid url");
        let url = notifier.request_url("Café ?&");
        assert_eq!(url.query(), Some("title=Caf%C3%A9+%3F%26"));
    }

    #[tokio::test]
    async fn accepted_notification_is_posted_with_title_and_body() {
        let (url, request) = serve_once("200 OK").await;
        let notifier = NtfyNotifier::new(&url, &topic()).expect("valid url");
        notifier.notify(&notification()).await.expect("accepted");
        let request = request.await.expect("server finishes");
        assert!(request.starts_with(&format!(
            "POST /{TOPIC}?title=bedroom+thermometer+offline HTTP/1.1"
        )));
        assert!(request.ends_with("\r\n\r\nNo reading for 1 h."));
    }

    #[tokio::test]
    async fn rejected_notification_fails_without_leaking_the_topic() {
        let (url, _request) = serve_once("500 Internal Server Error").await;
        let notifier = NtfyNotifier::new(&url, &topic()).expect("valid url");
        let error = notifier
            .notify(&notification())
            .await
            .expect_err("rejected");
        assert!(matches!(error, NtfyError::Send(_)));
        assert!(!format!("{error}").contains(TOPIC));
        assert!(!format!("{error:?}").contains(TOPIC));
    }

    #[tokio::test]
    async fn unreachable_server_fails_without_leaking_the_topic() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let closed = format!("http://{}", listener.local_addr().expect("has address"));
        drop(listener);
        let notifier = NtfyNotifier::new(&closed, &topic()).expect("valid url");
        let error = notifier.notify(&notification()).await.expect_err("refused");
        assert!(!format!("{error}").contains(TOPIC));
        assert!(!format!("{error:?}").contains(TOPIC));
    }
}
