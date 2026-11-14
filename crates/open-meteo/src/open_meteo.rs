use std::time::Duration;

use reqwest::{Client, Response, Url};
use use_cases::{WeatherProvider, WeatherReport};

use crate::OpenMeteoError;
use crate::forecast::parse;

const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetches current weather and today's forecast for one location.
pub struct OpenMeteo {
    http: Client,
    request_url: Url,
}

impl OpenMeteo {
    pub fn new(latitude: f64, longitude: f64) -> Self {
        let forecast_url = Url::parse(FORECAST_URL).expect("forecast url is valid");
        Self::with_url(forecast_url, latitude, longitude)
    }

    fn with_url(mut forecast_url: Url, latitude: f64, longitude: f64) -> Self {
        forecast_url
            .query_pairs_mut()
            .append_pair("latitude", &latitude.to_string())
            .append_pair("longitude", &longitude.to_string())
            .append_pair("current", "temperature_2m,relative_humidity_2m")
            .append_pair(
                "daily",
                "temperature_2m_max,temperature_2m_min,weather_code",
            )
            .append_pair("timezone", "Europe/Paris")
            .append_pair("forecast_days", "1")
            .append_pair("timeformat", "unixtime");
        Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("http client configuration is valid"),
            request_url: forecast_url,
        }
    }

    async fn fetch(&self) -> Result<Vec<u8>, reqwest::Error> {
        let response = self
            .http
            .get(self.request_url.clone())
            .send()
            .await
            .and_then(Response::error_for_status)?;
        Ok(response.bytes().await?.to_vec())
    }
}

impl WeatherProvider for OpenMeteo {
    type Error = OpenMeteoError;

    async fn current(&self) -> Result<WeatherReport, OpenMeteoError> {
        let body = self
            .fetch()
            .await
            .map_err(|error| OpenMeteoError::Request(error.without_url()))?;
        parse(&body)
    }
}

#[cfg(test)]
mod tests {
    use reqwest::Url;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use use_cases::WeatherProvider;

    use crate::OpenMeteoError;

    use super::OpenMeteo;

    const FIXTURE: &str = include_str!("../fixtures/forecast.json");

    async fn serve_once(status: &'static str, body: &'static str) -> (Url, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let url = format!(
            "http://{}/v1/forecast",
            listener.local_addr().expect("has address")
        );
        let request = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accepts");
            let mut received = Vec::new();
            while !received.ends_with(b"\r\n\r\n") {
                let mut chunk = [0; 1024];
                let read = stream.read(&mut chunk).await.expect("reads");
                received.extend_from_slice(&chunk[..read]);
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.expect("writes");
            String::from_utf8(received).expect("request is text")
        });
        (Url::parse(&url).expect("valid url"), request)
    }

    #[tokio::test]
    async fn current_requests_the_forecast_and_parses_it() {
        let (url, request) = serve_once("200 OK", FIXTURE).await;
        let provider = OpenMeteo::with_url(url, 48.8566, 2.3522);
        let report = provider.current().await.expect("fixture parses");
        assert_eq!(report.weather_code, 3);
        let request = request.await.expect("server finishes");
        let request_line = request.lines().next().expect("has a request line");
        for expected in [
            "GET /v1/forecast?",
            "latitude=48.8566",
            "longitude=2.3522",
            "current=temperature_2m%2Crelative_humidity_2m",
            "timezone=Europe%2FParis",
            "forecast_days=1",
            "timeformat=unixtime",
        ] {
            assert!(
                request_line.contains(expected),
                "{expected} in {request_line}"
            );
        }
    }

    #[tokio::test]
    async fn rejected_request_fails_without_leaking_the_location() {
        let (url, _request) = serve_once("400 Bad Request", "{\"error\":true}").await;
        let provider = OpenMeteo::with_url(url, 999.0, 2.3522);
        let error = provider.current().await.expect_err("rejected");
        assert!(matches!(error, OpenMeteoError::Request(_)));
        assert!(!format!("{error}").contains("999"));
        assert!(!format!("{error:?}").contains("999"));
    }

    #[test]
    fn default_url_is_https() {
        let provider = OpenMeteo::new(48.8566, 2.3522);
        assert_eq!(provider.request_url.scheme(), "https");
    }
}
