use std::time::Duration;

use domain::{ImportStatus, import_report};
use reqwest::{Client, Url};

/// Why the import status could not be read. The text names the category only,
/// so no URL, body or parser message reaches a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ImportPollError {
    #[error("the health collector is unreachable")]
    Unreachable,
    #[error("the health collector did not answer in time")]
    Timeout,
    #[error("the health collector sent an unusable answer")]
    BadResponse,
}

/// Reads the import status from the health collector.
pub struct ImportClient {
    http: Client,
    url: Url,
}

impl ImportClient {
    pub fn new(url: Url, timeout: Duration) -> Self {
        Self {
            http: Client::builder()
                .timeout(timeout)
                .build()
                .expect("http client configuration is valid"),
            url,
        }
    }

    pub async fn status(&self) -> Result<ImportStatus, ImportPollError> {
        let response = self
            .http
            .get(self.url.clone())
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(ImportPollError::BadResponse);
        }
        let body = response.bytes().await.map_err(transport)?;
        import_report::decode(&body).map_err(|_| ImportPollError::BadResponse)
    }
}

fn transport(error: reqwest::Error) -> ImportPollError {
    if error.is_timeout() {
        ImportPollError::Timeout
    } else {
        ImportPollError::Unreachable
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use axum::Router;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use domain::{ImportStatus, Timestamp, import_report};
    use reqwest::Url;
    use tokio::net::TcpListener;

    use super::{ImportClient, ImportPollError};

    const SENTINEL: &str = "SENTINEL_BODY_TEXT";

    pub(crate) async fn serve(router: Router) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let port = listener.local_addr().expect("address").port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serves");
        });
        Url::parse(&format!("http://127.0.0.1:{port}/imports")).expect("valid url")
    }

    fn client(url: Url) -> ImportClient {
        ImportClient::new(url, Duration::from_millis(200))
    }

    #[tokio::test]
    async fn a_valid_report_is_decoded() {
        let status = ImportStatus {
            next_pull: Some(Timestamp::from_micros(5)),
            ..ImportStatus::default()
        };
        let body = import_report::encode(&status);
        let url = serve(Router::new().route("/imports", get(move || async move { body }))).await;
        assert_eq!(client(url).status().await, Ok(status));
    }

    #[tokio::test]
    async fn a_handler_slower_than_the_timeout_is_a_timeout() {
        let slow = get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "late"
        });
        let url = serve(Router::new().route("/imports", slow)).await;
        assert_eq!(client(url).status().await, Err(ImportPollError::Timeout));
    }

    #[tokio::test]
    async fn a_server_that_is_gone_is_unreachable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let port = listener.local_addr().expect("address").port();
        drop(listener);
        let url = Url::parse(&format!("http://127.0.0.1:{port}/imports")).expect("valid url");
        assert_eq!(
            client(url).status().await,
            Err(ImportPollError::Unreachable)
        );
    }

    #[tokio::test]
    async fn a_non_success_status_is_a_bad_response_even_with_a_valid_report() {
        let body = import_report::encode(&ImportStatus::default());
        let handler = get(move || async move { (StatusCode::MULTIPLE_CHOICES, body) });
        let url = serve(Router::new().route("/imports", handler)).await;
        assert_eq!(
            client(url).status().await,
            Err(ImportPollError::BadResponse)
        );
    }

    #[tokio::test]
    async fn an_error_status_and_a_malformed_body_are_bad_responses_that_keep_the_body_out() {
        let failing =
            get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, SENTINEL).into_response() });
        let malformed = get(|| async { format!(r#"{{"version":1,"jobs":"{SENTINEL}"}}"#) });
        for handler in [failing, malformed] {
            let url = serve(Router::new().route("/imports", handler)).await;
            let error = client(url).status().await.expect_err("unusable");
            assert_eq!(error, ImportPollError::BadResponse);
            assert!(!error.to_string().contains(SENTINEL));
            assert!(!format!("{error:?}").contains(SENTINEL));
            assert!(!error.to_string().contains("127.0.0.1"));
        }
    }
}
