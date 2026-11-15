use reqwest::Response;
use serde::de::DeserializeOwned;

use crate::GarminError;

/// Reads a JSON body. A decoding failure becomes an error that names the
/// endpoint only, because serde's message can echo values from the body.
pub async fn read_json<T: DeserializeOwned>(
    response: Response,
    endpoint: &'static str,
) -> Result<T, GarminError> {
    let bytes = response.bytes().await.map_err(GarminError::request)?;
    serde_json::from_slice(&bytes).map_err(|_| GarminError::InvalidResponse { endpoint })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use reqwest::Client;

    use crate::GarminError;
    use crate::test_server::{Reply, TestServer};

    use super::read_json;

    #[derive(Debug, serde::Deserialize)]
    struct Summary {
        #[serde(rename = "totalSteps")]
        _steps: Option<u32>,
    }

    async fn read(body: &str) -> Result<Summary, GarminError> {
        let server = TestServer::start(vec![Reply::new(200, body)]).await;
        let response = Client::new()
            .get(server.base.clone())
            .send()
            .await
            .expect("response");
        read_json(response, "daily summary").await
    }

    #[tokio::test]
    async fn a_valid_body_decodes() {
        let summary = read(r#"{"totalSteps":5}"#).await.expect("decodes");
        assert_eq!(summary._steps, Some(5));
    }

    #[tokio::test]
    async fn decoding_errors_name_the_endpoint_and_nothing_from_the_body() {
        for body in [r#"{"totalSteps":"secret-material"}"#, "secret-material"] {
            let error = read(body).await.expect_err("does not decode");
            assert!(matches!(
                error,
                GarminError::InvalidResponse {
                    endpoint: "daily summary"
                }
            ));
            let mut rendered = format!("{error} {error:?}");
            let mut source = error.source();
            while let Some(cause) = source {
                rendered.push_str(&format!(" {cause} {cause:?}"));
                source = cause.source();
            }
            assert!(error.source().is_none());
            assert!(!rendered.contains("secret-material"));
        }
    }
}
