use reqwest::StatusCode;
use use_cases::{ActivityRef, ArchiveSource, DocumentKey, Fetched};

use crate::activity_list::{ActivityList, ENDPOINT};
use crate::endpoints::build_url;
use crate::garmin_document::{ACTIVITY, DAILY};
use crate::read_json::read_json;
use crate::{GarminConnect, GarminDocument, GarminError, GarminSession, JSON};

impl ArchiveSource for GarminConnect {
    type Document = GarminDocument;

    const DAILY: &'static [GarminDocument] = DAILY;
    const ACTIVITY: &'static [GarminDocument] = ACTIVITY;

    fn file_name(document: GarminDocument) -> &'static str {
        document.file_name()
    }

    async fn document(
        &self,
        session: &GarminSession,
        document: GarminDocument,
        key: DocumentKey,
    ) -> Result<Fetched, GarminError> {
        let key = match key {
            DocumentKey::Day(date) => date.to_string(),
            DocumentKey::Activity(id) => id.to_string(),
        };
        let url = document.url(&self.endpoints.api, &session.display_name, &key);
        let response = self
            .send_get(&session.tokens.di_token, url, document.accept())
            .await?;
        match response.status() {
            status @ StatusCode::UNAUTHORIZED => Err(GarminError::SessionRejected(status)),
            StatusCode::FORBIDDEN => Ok(Fetched::Denied),
            StatusCode::NOT_FOUND => Ok(Fetched::Absent),
            status if status.is_success() => {
                let bytes = response.bytes().await.map_err(GarminError::request)?;
                Ok(Fetched::Body(bytes.to_vec()))
            }
            status => Err(GarminError::Status(status)),
        }
    }

    async fn activities(
        &self,
        session: &GarminSession,
        start: u32,
        limit: u32,
    ) -> Result<Vec<ActivityRef>, GarminError> {
        let url = build_url(
            &self.endpoints.api,
            &["activitylist-service", "activities", "search", "activities"],
            &[("start", &start.to_string()), ("limit", &limit.to_string())],
        );
        let response = self.send_get(&session.tokens.di_token, url, JSON).await?;
        match response.status() {
            status @ StatusCode::UNAUTHORIZED => Err(GarminError::SessionRejected(status)),
            status if !status.is_success() => Err(GarminError::Status(status)),
            _ => {
                let list: ActivityList = read_json(response, ENDPOINT).await?;
                list.into_refs()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use tempfile::TempDir;
    use use_cases::{ActivityId, ArchiveSource, DocumentKey, Fetched, HealthProvider};

    use crate::garmin_connect::tests::{DI_TOKEN, PROFILE, connect};
    use crate::test_server::Reply;
    use crate::{GarminConnect, GarminError};

    fn day() -> DocumentKey {
        DocumentKey::Day(NaiveDate::from_ymd_opt(2026, 7, 14).expect("date"))
    }

    fn opening() -> Vec<Reply> {
        vec![Reply::new(200, DI_TOKEN), Reply::new(200, PROFILE)]
    }

    /// Opens a session, then answers one `document` call with `reply`.
    async fn document(reply: Reply) -> Result<Fetched, GarminError> {
        let dir = TempDir::new().expect("tempdir");
        let mut replies = opening();
        replies.push(reply);
        let (connect, _server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        connect
            .document(&session, GarminConnect::DAILY[0], day())
            .await
    }

    async fn list(reply: Reply) -> Result<Vec<use_cases::ActivityRef>, GarminError> {
        let dir = TempDir::new().expect("tempdir");
        let mut replies = opening();
        replies.push(reply);
        let (connect, _server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        connect.activities(&session, 10, 5).await
    }

    #[tokio::test]
    async fn every_document_is_requested_with_its_accept_header_and_the_token() {
        let dir = TempDir::new().expect("tempdir");
        let mut replies = opening();
        replies.extend((0..60).map(|_| Reply::new(200, "{}")));
        let (connect, server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        let daily = GarminConnect::DAILY
            .iter()
            .map(|document| (document, day()));
        let activity = GarminConnect::ACTIVITY
            .iter()
            .map(|document| (document, DocumentKey::Activity(ActivityId(123))));
        let calls: Vec<_> = daily.chain(activity).collect();
        for (document, key) in &calls {
            connect
                .document(&session, **document, *key)
                .await
                .expect("document");
        }
        let requests = server.requests();
        assert_eq!(requests.len(), 2 + 60);
        for ((document, _), request) in calls.iter().zip(&requests[2..]) {
            let accept = if GarminConnect::file_name(**document).ends_with(".zip") {
                "accept: */*"
            } else {
                "accept: application/json"
            };
            assert!(request.has_header(accept));
            assert!(request.has_header("authorization: Bearer new-access-token"));
        }
        assert_eq!(
            requests[2].request_line(),
            "GET /usersummary-service/usersummary/daily/Jane%20Doe?calendarDate=2026-07-14 HTTP/1.1"
        );
        assert_eq!(
            requests[2 + 43 + 3].request_line(),
            "GET /activity-service/activity/123/splits HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn the_body_is_kept_as_delivered() {
        let body = "{ \"a\" :1 }\n\n";
        let fetched = document(Reply::new(200, body)).await.expect("fetched");
        assert_eq!(fetched, Fetched::Body(body.as_bytes().to_vec()));
    }

    #[tokio::test]
    async fn no_content_is_an_empty_body_and_404_and_403_are_told_apart() {
        assert_eq!(
            document(Reply::status(204)).await.expect("204"),
            Fetched::Body(Vec::new())
        );
        assert_eq!(
            document(Reply::status(404)).await.expect("404"),
            Fetched::Absent
        );
        assert_eq!(
            document(Reply::status(403)).await.expect("403"),
            Fetched::Denied
        );
    }

    #[tokio::test]
    async fn a_refused_session_throttling_and_failures_are_told_apart() {
        for (status, session, throttled) in [
            (401, true, false),
            (427, false, true),
            (429, false, true),
            (500, false, false),
        ] {
            let error = document(Reply::status(status)).await.expect_err("fails");
            assert_eq!(GarminConnect::is_session_error(&error), session, "{status}");
            assert_eq!(GarminConnect::is_throttled(&error), throttled, "{status}");
        }
    }

    #[tokio::test]
    async fn the_list_is_requested_by_offset_and_page_size() {
        let dir = TempDir::new().expect("tempdir");
        let mut replies = opening();
        replies.push(Reply::new(200, "[]"));
        let (connect, server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        connect.activities(&session, 10, 5).await.expect("list");
        assert_eq!(
            server.requests()[2].request_line(),
            "GET /activitylist-service/activities/search/activities?start=10&limit=5 HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn a_refused_list_is_an_error_and_only_401_is_a_session_error() {
        let forbidden = list(Reply::status(403)).await.expect_err("403");
        assert!(matches!(forbidden, GarminError::Status(_)));
        assert!(!GarminConnect::is_session_error(&forbidden));
        let unauthorized = list(Reply::status(401)).await.expect_err("401");
        assert!(GarminConnect::is_session_error(&unauthorized));
    }

    #[tokio::test]
    async fn nothing_secret_reaches_errors_or_debug_output() {
        let dir = TempDir::new().expect("tempdir");
        let mut replies = opening();
        replies.push(Reply::new(500, "body-with-secret-material"));
        let (connect, server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        let error = connect
            .document(&session, GarminConnect::DAILY[0], day())
            .await
            .expect_err("fails");
        let rendered = format!("{error} {error:?} {:?}", GarminConnect::DAILY[0]);
        for secret in [
            "new-access-token",
            "body-with-secret-material",
            "Jane",
            server.base.as_str(),
        ] {
            assert!(!rendered.contains(secret), "{secret} leaked");
        }
    }
}
