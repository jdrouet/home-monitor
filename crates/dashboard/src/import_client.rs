use std::time::Duration;

use domain::{ImportStatus, import_report};
use reqwest::{Client, Url};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Why the import status could not be read. The text names the category only,
/// so no URL, body or parser message reaches a page or a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ImportClientError {
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
    imports_url: Url,
}

impl ImportClient {
    pub fn new(imports_url: Url) -> Self {
        Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("http client configuration is valid"),
            imports_url,
        }
    }

    pub async fn status(&self) -> Result<ImportStatus, ImportClientError> {
        let response = self
            .http
            .get(self.imports_url.clone())
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(ImportClientError::BadResponse);
        }
        let body = response.bytes().await.map_err(transport)?;
        import_report::decode(&body).map_err(|_| ImportClientError::BadResponse)
    }
}

fn transport(error: reqwest::Error) -> ImportClientError {
    if error.is_timeout() {
        ImportClientError::Timeout
    } else {
        ImportClientError::Unreachable
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use domain::{ImportStatus, import_report};
    use reqwest::Url;
    use tokio::net::TcpListener;

    use super::{ImportClient, ImportClientError};

    #[tokio::test]
    async fn a_non_success_status_is_a_bad_response_even_with_a_valid_report() {
        let body = import_report::encode(&ImportStatus::default());
        let handler = get(move || async move { (StatusCode::MULTIPLE_CHOICES, body) });
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let port = listener.local_addr().expect("address").port();
        let router = Router::new().route("/imports", handler);
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serves");
        });
        let url = Url::parse(&format!("http://127.0.0.1:{port}/imports")).expect("valid url");
        assert_eq!(
            ImportClient::new(url).status().await,
            Err(ImportClientError::BadResponse)
        );
    }
}
