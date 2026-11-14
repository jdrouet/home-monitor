use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use domain::{SeriesQuery, batch};
use use_cases::MeasurementStore;

pub fn router<S: MeasurementStore + Send + Sync + 'static>(store: Arc<S>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/latest", get(latest::<S>))
        .route("/series", get(series::<S>))
        .with_state(store)
}

async fn latest<S: MeasurementStore + Send + Sync + 'static>(
    State(store): State<Arc<S>>,
) -> Response {
    json(batch::encode(&store.latest()))
}

async fn series<S: MeasurementStore + Send + Sync + 'static>(
    State(store): State<Arc<S>>,
    Query(query): Query<SeriesQuery>,
) -> Response {
    match store.series(&query).await {
        Ok(rows) => json(batch::encode(&rows)),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

fn json(body: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::num::NonZeroU32;
    use std::sync::{Arc, Mutex};

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;
    use domain::{MacAddress, Measurement, Metric, SeriesQuery, SourceKind, Timestamp};
    use tower::ServiceExt;
    use use_cases::MeasurementStore;

    use super::router;

    #[derive(Default)]
    struct FakeStore {
        latest: Vec<Measurement>,
        series: Vec<Measurement>,
        fail: bool,
        seen: Mutex<Option<SeriesQuery>>,
    }

    impl MeasurementStore for FakeStore {
        type Error = io::Error;

        fn append(&self, _batch: Vec<Measurement>) -> Result<(), io::Error> {
            Ok(())
        }

        fn latest(&self) -> Vec<Measurement> {
            self.latest.clone()
        }

        async fn series(&self, query: &SeriesQuery) -> Result<Vec<Measurement>, io::Error> {
            *self.seen.lock().expect("lock") = Some(query.clone());
            if self.fail {
                return Err(io::Error::other("disk on fire"));
            }
            Ok(self.series.clone())
        }

        async fn flush_due(&self, _now: Timestamp) -> Result<(), io::Error> {
            Ok(())
        }
    }

    fn sample() -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(5),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([0xA4, 0xC1, 0x38, 0x1C, 0x02, 0x76]).into(),
            metric: Metric::Temperature,
            value: 21.27,
        }
    }

    async fn get(store: &Arc<FakeStore>, uri: &str) -> Response {
        router(Arc::clone(store))
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
        let response = get(&Arc::default(), "/latest").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            body_text(response).await,
            r#"{"version":1,"measurements":[]}"#
        );
    }

    #[tokio::test]
    async fn latest_values_are_served() {
        let store = Arc::new(FakeStore {
            latest: vec![sample()],
            ..FakeStore::default()
        });
        let text = body_text(get(&store, "/latest").await).await;
        assert!(text.contains(r#""source_id":"A4:C1:38:1C:02:76""#));
        assert!(text.contains(r#""value":21.27"#));
    }

    #[tokio::test]
    async fn series_passes_every_parameter_and_serves_the_rows() {
        let store = Arc::new(FakeStore {
            series: vec![sample()],
            ..FakeStore::default()
        });
        let uri =
            "/series?metric=humidity&from=10&to=20&source_id=A4:C1:38:1C:02:76&bucket_secs=60";
        let response = get(&store, uri).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert!(body_text(response).await.contains(r#""value":21.27"#));
        let seen = store.seen.lock().expect("lock").clone().expect("queried");
        assert_eq!(seen.metric, Metric::Humidity);
        assert_eq!(seen.from, Timestamp::from_micros(10));
        assert_eq!(seen.to, Timestamp::from_micros(20));
        assert_eq!(
            seen.source_id.as_ref().map(|id| id.as_str()),
            Some("A4:C1:38:1C:02:76")
        );
        assert_eq!(seen.bucket_secs, NonZeroU32::new(60));
    }

    #[tokio::test]
    async fn a_failing_store_is_a_500_with_the_error_text() {
        let store = Arc::new(FakeStore {
            fail: true,
            ..FakeStore::default()
        });
        let response = get(&store, "/series?metric=temperature&from=0&to=1").await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_text(response).await, "disk on fire");
    }

    #[tokio::test]
    async fn bad_parameters_are_a_400() {
        let store = Arc::default();
        for uri in [
            "/series?metric=bogus&from=0&to=1",
            "/series?metric=temperature&from=0&to=1&bucket_secs=0",
            "/series?metric=temperature&from=0",
        ] {
            assert_eq!(get(&store, uri).await.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn healthz_is_ok() {
        let response = get(&Arc::default(), "/healthz").await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
