use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use domain::batch;
use use_cases::LatestValues;

type SharedValues = Arc<Mutex<LatestValues>>;

pub fn router(state: SharedValues) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/latest", get(latest))
        .with_state(state)
}

async fn latest(State(state): State<SharedValues>) -> impl IntoResponse {
    let body = batch::encode(&state.lock().expect("state lock poisoned").snapshot());
    ([(header::CONTENT_TYPE, "application/json")], body)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;
    use domain::{MacAddress, Measurement, Metric, SourceKind, Timestamp};
    use tower::ServiceExt;
    use use_cases::LatestValues;

    use super::router;

    async fn get(state: Arc<Mutex<LatestValues>>, uri: &str) -> Response {
        router(state)
            .oneshot(
                Request::get(uri)
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router is infallible")
    }

    async fn body_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        String::from_utf8(bytes.to_vec()).expect("utf-8 body")
    }

    #[tokio::test]
    async fn empty_store_serves_an_empty_batch() {
        let response = get(Arc::default(), "/latest").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            body_text(response).await,
            r#"{"version":1,"measurements":[]}"#
        );
    }

    #[tokio::test]
    async fn recorded_values_are_served() {
        let state = Arc::new(Mutex::new(LatestValues::default()));
        state.lock().expect("lock").record(vec![Measurement {
            timestamp: Timestamp::from_micros(5),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([0xA4, 0xC1, 0x38, 0x1C, 0x02, 0x76]).into(),
            metric: Metric::Temperature,
            value: 21.27,
        }]);
        let text = body_text(get(state, "/latest").await).await;
        assert!(text.contains(r#""source_id":"A4:C1:38:1C:02:76""#));
        assert!(text.contains(r#""value":21.27"#));
    }

    #[tokio::test]
    async fn healthz_is_ok() {
        let response = get(Arc::default(), "/healthz").await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
