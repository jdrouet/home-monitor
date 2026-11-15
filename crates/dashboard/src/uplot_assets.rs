use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;

// The file name carries the uPlot version, so a cached copy never goes stale.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub async fn script() -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, "text/javascript"),
            (CACHE_CONTROL, IMMUTABLE),
        ],
        include_str!("../assets/uPlot.iife.min.js"),
    )
}

pub async fn stylesheet() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/css"), (CACHE_CONTROL, IMMUTABLE)],
        include_str!("../assets/uPlot.min.css"),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
    use axum::response::{IntoResponse, Response};

    use super::{script, stylesheet};

    async fn body(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        String::from_utf8(bytes.to_vec()).expect("utf-8 body")
    }

    #[tokio::test]
    async fn the_script_is_served_as_immutable_javascript() {
        let response = script().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/javascript");
        assert_eq!(
            response.headers()[CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        let text = body(response).await;
        assert!(text.starts_with("/*! https://github.com/leeoniya/uPlot (v1.6.32)"));
    }

    #[tokio::test]
    async fn the_stylesheet_is_served_as_immutable_css() {
        let response = stylesheet().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/css");
        assert_eq!(
            response.headers()[CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        assert!(body(response).await.starts_with(".uplot"));
    }
}
