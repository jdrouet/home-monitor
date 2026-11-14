use std::future::Future;

use crate::WeatherReport;

/// Fetches the weather for a location fixed when the provider is built.
pub trait WeatherProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn current(&self) -> impl Future<Output = Result<WeatherReport, Self::Error>> + Send;
}
