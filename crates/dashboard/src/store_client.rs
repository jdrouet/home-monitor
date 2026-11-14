use domain::{BatchError, Measurement, batch};
use reqwest::{Client, Url};

#[derive(Debug, thiserror::Error)]
pub enum StoreClientError {
    #[error("store request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("store returned an invalid batch: {0}")]
    Batch(#[from] BatchError),
}

/// Reads the latest measurements from the store service.
pub struct StoreClient {
    http: Client,
    latest_url: Url,
}

impl StoreClient {
    pub fn new(latest_url: Url) -> Self {
        Self {
            http: Client::new(),
            latest_url,
        }
    }

    pub async fn latest(&self) -> Result<Vec<Measurement>, StoreClientError> {
        Ok(batch::decode(&self.fetch().await?)?)
    }

    async fn fetch(&self) -> Result<Vec<u8>, reqwest::Error> {
        let response = self.http.get(self.latest_url.clone()).send().await?;
        Ok(response.error_for_status()?.bytes().await?.to_vec())
    }
}
