use std::time::Duration;

use domain::{BatchError, Measurement, SeriesQuery, batch};
use reqwest::{Client, Url};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum StoreClientError {
    #[error("store request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("store returned an invalid batch: {0}")]
    Batch(#[from] BatchError),
}

/// Reads measurements from the store service.
pub struct StoreClient {
    http: Client,
    latest_url: Url,
    series_url: Url,
}

impl StoreClient {
    pub fn new(latest_url: Url, series_url: Url) -> Self {
        Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("http client configuration is valid"),
            latest_url,
            series_url,
        }
    }

    pub async fn latest(&self) -> Result<Vec<Measurement>, StoreClientError> {
        Ok(batch::decode(&self.fetch(self.latest_url.clone()).await?)?)
    }

    pub async fn series(&self, query: &SeriesQuery) -> Result<Vec<Measurement>, StoreClientError> {
        let url = series_url(&self.series_url, query);
        Ok(batch::decode(&self.fetch(url).await?)?)
    }

    async fn fetch(&self, url: Url) -> Result<Vec<u8>, reqwest::Error> {
        let response = self.http.get(url).send().await?;
        Ok(response.error_for_status()?.bytes().await?.to_vec())
    }
}

fn series_url(base: &Url, query: &SeriesQuery) -> Url {
    let mut url = base.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("metric", query.metric.as_str())
            .append_pair("from", &query.from.as_micros().to_string())
            .append_pair("to", &query.to.as_micros().to_string());
        if let Some(source_id) = &query.source_id {
            pairs.append_pair("source_id", source_id.as_str());
        }
        if let Some(bucket_secs) = query.bucket_secs {
            pairs.append_pair("bucket_secs", &bucket_secs.to_string());
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use domain::{Metric, SeriesQuery, Timestamp};
    use reqwest::Url;

    use super::series_url;

    fn query() -> SeriesQuery {
        SeriesQuery {
            metric: Metric::Humidity,
            source_id: None,
            from: Timestamp::from_micros(10),
            to: Timestamp::from_micros(20),
            bucket_secs: None,
        }
    }

    fn base() -> Url {
        Url::parse("http://store:8080/series").expect("valid url")
    }

    #[test]
    fn a_plain_query_carries_the_metric_and_the_range() {
        assert_eq!(
            series_url(&base(), &query()).as_str(),
            "http://store:8080/series?metric=humidity&from=10&to=20"
        );
    }

    #[test]
    fn optional_fields_are_added_and_escaped() {
        let query = SeriesQuery {
            source_id: Some(String::from("A4:C1 38").into()),
            bucket_secs: NonZeroU32::new(900),
            ..query()
        };
        assert_eq!(
            series_url(&base(), &query).as_str(),
            "http://store:8080/series?metric=humidity&from=10&to=20&source_id=A4%3AC1+38&bucket_secs=900"
        );
    }
}
