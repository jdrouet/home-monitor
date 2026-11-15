use std::time::Duration;

use domain::{AlertReportError, AlertState, alert_report};
use reqwest::{Client, Url};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum AlerterClientError {
    #[error("alerter request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("alerter returned an invalid report: {0}")]
    Report(#[from] AlertReportError),
}

/// Reads the alert report from the alerter service.
pub struct AlerterClient {
    http: Client,
    alerts_url: Url,
}

impl AlerterClient {
    pub fn new(alerts_url: Url) -> Self {
        Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("http client configuration is valid"),
            alerts_url,
        }
    }

    pub async fn alerts(&self) -> Result<AlertState, AlerterClientError> {
        let response = self.http.get(self.alerts_url.clone()).send().await?;
        let body = response.error_for_status()?.bytes().await?;
        Ok(alert_report::decode(&body)?)
    }
}
