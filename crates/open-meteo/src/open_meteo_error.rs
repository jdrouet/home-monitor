#[derive(Debug, thiserror::Error)]
pub enum OpenMeteoError {
    #[error("open-meteo request failed: {0}")]
    Request(reqwest::Error),
    #[error("open-meteo response is not a valid forecast: {0}")]
    Json(#[from] serde_json::Error),
    #[error("open-meteo response has no forecast for today")]
    MissingForecast,
}
