use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use domain::{AlertState, alert_report};
use tokio::sync::watch;

pub fn router(report: watch::Receiver<AlertState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/alerts", get(alerts))
        .with_state(report)
}

async fn alerts(State(report): State<watch::Receiver<AlertState>>) -> Response {
    let body = alert_report::encode(&report.borrow());
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;
    use domain::{
        AlertKind, AlertState, OfflineAlert, ResolvedAlert, SourceId, Timestamp, alert_report,
    };
    use tokio::sync::watch;
    use tower::ServiceExt;

    use super::router;

    fn state() -> AlertState {
        let mut state = AlertState::default();
        state.offline.insert(
            SourceId::from("A4:C1:38:00:00:01".to_owned()),
            OfflineAlert {
                last_seen: Timestamp::from_micros(10),
                since: Timestamp::from_micros(20),
                notified_at: Timestamp::from_micros(30),
            },
        );
        state.history.push(ResolvedAlert {
            source_id: SourceId::from("A4:C1:38:00:00:02".to_owned()),
            kind: AlertKind::TooHot,
            started: Timestamp::from_micros(40),
            ended: Timestamp::from_micros(50),
        });
        state
    }

    async fn get(uri: &str) -> Response {
        let (_sender, receiver) = watch::channel(state());
        router(receiver)
            .oneshot(
                Request::get(uri)
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router is infallible")
    }

    #[tokio::test]
    async fn alerts_are_served_as_the_report() {
        let response = get("/alerts").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        assert_eq!(body.to_vec(), alert_report::encode(&state()));
    }

    #[tokio::test]
    async fn healthz_is_ok() {
        assert_eq!(get("/healthz").await.status(), StatusCode::OK);
    }
}
