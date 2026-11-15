use std::io;
use std::time::Duration;

use chrono::NaiveDate;
use reqwest::header::{ACCEPT, CACHE_CONTROL, HeaderMap, LOCATION};
use reqwest::redirect::Policy;
use reqwest::{Client, RequestBuilder, Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use tokio::task::spawn_blocking;
use use_cases::{DailyHealth, HealthProvider};

use crate::daily_sleep::DailySleep;
use crate::daily_summary::DailySummary;
use crate::endpoints::build_url;
use crate::native_headers::native_headers;
use crate::pacer::Pacer;
use crate::read_json::read_json;
use crate::social_profile::SocialProfile;
use crate::token_response::TokenResponse;
use crate::{Endpoints, GarminError, GarminSession, TokenFile, Tokens};

const TIMEOUT: Duration = Duration::from_secs(30);
const API_REDIRECTS: u8 = 3;

/// Every request of the process, redirect hops included, waits on this pacer.
static PACER: Pacer = Pacer::new(Duration::from_secs(3)..=Duration::from_secs(6));

/// Garmin Connect's unofficial API, reached with the tokens in a token file.
pub struct GarminConnect {
    pub(crate) http: Client,
    pub(crate) endpoints: Endpoints,
    pub(crate) token_file: TokenFile,
    pub(crate) pacer: &'static Pacer,
}

impl GarminConnect {
    pub fn new(token_file: TokenFile) -> Result<Self, GarminError> {
        Self::with_endpoints(token_file, Endpoints::garmin(), &PACER)
    }

    pub(crate) fn with_endpoints(
        token_file: TokenFile,
        endpoints: Endpoints,
        pacer: &'static Pacer,
    ) -> Result<Self, GarminError> {
        let http = Client::builder()
            .timeout(TIMEOUT)
            .redirect(Policy::none())
            .build()
            .map_err(GarminError::request)?;
        Ok(Self {
            http,
            endpoints,
            token_file,
            pacer,
        })
    }

    /// GETs `first`, following up to `max_hops` redirects by hand so that each
    /// hop waits on the pacer. `prepare` shapes every request; it gets the
    /// previous hop's headers, `None` on the first request. A redirect past
    /// the limit is returned as is.
    pub(crate) async fn get_following(
        &self,
        first: Url,
        max_hops: u8,
        mut prepare: impl FnMut(RequestBuilder, &Url, Option<&HeaderMap>) -> RequestBuilder,
    ) -> Result<Response, GarminError> {
        let mut url = first;
        let mut previous: Option<HeaderMap> = None;
        let mut hops = 0;
        loop {
            self.pacer.wait().await;
            let request = prepare(self.http.get(url.clone()), &url, previous.as_ref());
            let response = request.send().await.map_err(GarminError::request)?;
            match redirect_target(&response, &url) {
                Some(next) if hops < max_hops => {
                    previous = Some(response.headers().clone());
                    url = next;
                    hops += 1;
                }
                _ => return Ok(response),
            }
        }
    }

    /// GETs `url` as the signed-in user. The token goes only to the API host.
    pub(crate) async fn send_get(
        &self,
        token: &str,
        url: Url,
        accept: &'static str,
    ) -> Result<Response, GarminError> {
        let api = self.endpoints.api.origin();
        self.get_following(url, API_REDIRECTS, |request, url, _| {
            let request = request.headers(native_headers()).header(ACCEPT, accept);
            if url.origin() == api {
                request.bearer_auth(token)
            } else {
                request
            }
        })
        .await
    }

    /// Refreshes the stored tokens and saves the result before any other call.
    /// The lock is held from the load to the save and is taken off the runtime
    /// thread, which other lock holders' requests may need. The save's task
    /// owns the lock, so a cancelled call cannot release it mid-write.
    async fn refresh_and_save(&self) -> Result<Tokens, GarminError> {
        let file = self.token_file.clone();
        let (lock, stored) = blocking(move || Ok((file.lock()?, file.load()?))).await?;
        let fresh = self.refresh(&stored).await?;
        let file = self.token_file.clone();
        blocking(move || {
            let _lock = lock;
            file.save(&fresh).map(|()| fresh)
        })
        .await
    }

    async fn refresh(&self, stored: &Tokens) -> Result<Tokens, GarminError> {
        self.pacer.wait().await;
        let response = self
            .http
            .post(self.endpoints.token.clone())
            .headers(native_headers())
            .basic_auth(&stored.di_client_id, Some(""))
            .header(ACCEPT, "application/json")
            .header(CACHE_CONTROL, "no-cache")
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", stored.di_client_id.as_str()),
                ("refresh_token", stored.di_refresh_token.as_str()),
            ])
            .send()
            .await
            .map_err(GarminError::request)?;
        let status = response.status();
        if !status.is_success() {
            return Err(GarminError::Refresh(status));
        }
        let body: TokenResponse = read_json(response, "token refresh").await?;
        body.into_tokens(stored.di_client_id.clone(), Some(&stored.di_refresh_token))
    }

    /// GETs `url` as the signed-in user. A 401 or 403 means the session is
    /// refused.
    async fn fetch<T: DeserializeOwned>(
        &self,
        token: &str,
        url: Url,
        endpoint: &'static str,
    ) -> Result<T, GarminError> {
        let response = self.send_get(token, url, "application/json").await?;
        match response.status() {
            status @ (StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => {
                Err(GarminError::SessionRejected(status))
            }
            status if !status.is_success() => Err(GarminError::Status(status)),
            _ => read_json(response, endpoint).await,
        }
    }

    pub(crate) async fn display_name(&self, tokens: &Tokens) -> Result<String, GarminError> {
        let url = build_url(
            &self.endpoints.api,
            &["userprofile-service", "socialProfile"],
            &[],
        );
        let profile: SocialProfile = self.fetch(&tokens.di_token, url, "social profile").await?;
        profile.display_name()
    }
}

/// Where a redirect response points, resolved against the URL it answered.
fn redirect_target(response: &Response, from: &Url) -> Option<Url> {
    if !response.status().is_redirection() {
        return None;
    }
    let location = response.headers().get(LOCATION)?.to_str().ok()?;
    from.join(location).ok()
}

/// Runs token file I/O, which blocks, off the async runtime.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, GarminError> + Send + 'static,
) -> Result<T, GarminError> {
    spawn_blocking(work)
        .await
        .expect("token file task does not panic")
}

impl HealthProvider for GarminConnect {
    type Session = GarminSession;
    type Error = GarminError;

    async fn open(&self) -> Result<GarminSession, GarminError> {
        let tokens = self.refresh_and_save().await?;
        let display_name = self.display_name(&tokens).await?;
        Ok(GarminSession {
            tokens,
            display_name,
        })
    }

    async fn daily(
        &self,
        session: &GarminSession,
        date: NaiveDate,
    ) -> Result<DailyHealth, GarminError> {
        let day = date.to_string();
        let token = &session.tokens.di_token;
        let name = session.display_name.as_str();
        let summary_url = build_url(
            &self.endpoints.api,
            &["usersummary-service", "usersummary", "daily", name],
            &[("calendarDate", &day)],
        );
        let sleep_url = build_url(
            &self.endpoints.api,
            &["wellness-service", "wellness", "dailySleepData", name],
            &[("date", &day), ("nonSleepBufferMinutes", "60")],
        );
        let summary: DailySummary = self.fetch(token, summary_url, "daily summary").await?;
        let sleep: DailySleep = self.fetch(token, sleep_url, "daily sleep").await?;
        Ok(DailyHealth {
            sleep_seconds: sleep.seconds(),
            sleep_score: sleep.score(),
            resting_heart_rate: summary.resting_heart_rate,
            steps: summary.steps,
            body_battery_max: summary.body_battery_max,
            body_battery_min: summary.body_battery_min,
        })
    }

    fn is_session_error(error: &GarminError) -> bool {
        match error {
            GarminError::SessionRejected(_) | GarminError::InvalidTokenFile { .. } => true,
            GarminError::Refresh(status) => matches!(status.as_u16(), 400 | 401 | 403),
            GarminError::TokenFile { source, .. } => source.kind() == io::ErrorKind::NotFound,
            _ => false,
        }
    }

    fn is_throttled(error: &GarminError) -> bool {
        matches!(
            error,
            GarminError::Status(status) | GarminError::Refresh(status)
                if matches!(status.as_u16(), 427 | 429)
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::io;
    use std::time::Duration;

    use chrono::NaiveDate;
    use tempfile::TempDir;
    use use_cases::HealthProvider;

    use crate::pacer::{NO_GAP, Pacer};
    use crate::test_server::{Reply, TestServer};
    use crate::{Endpoints, GarminConnect, GarminError, TokenFile, Tokens};

    pub(crate) const DI_TOKEN: &str = include_str!("../fixtures/di_token.json");
    pub(crate) const PROFILE: &str = include_str!("../fixtures/social_profile.json");
    const SUMMARY: &str = include_str!("../fixtures/user_summary.json");
    const SLEEP: &str = include_str!("../fixtures/daily_sleep.json");

    fn stored() -> Tokens {
        Tokens {
            di_token: "old-access".to_owned(),
            di_refresh_token: "old-refresh".to_owned(),
            di_client_id: "client".to_owned(),
        }
    }

    pub(crate) async fn connect(
        replies: Vec<Reply>,
        dir: &TempDir,
    ) -> (GarminConnect, TestServer, TokenFile) {
        connect_paced(replies, dir, &NO_GAP).await
    }

    async fn connect_paced(
        replies: Vec<Reply>,
        dir: &TempDir,
        pacer: &'static Pacer,
    ) -> (GarminConnect, TestServer, TokenFile) {
        let server = TestServer::start(replies).await;
        let endpoints = Endpoints {
            sso: server.base.clone(),
            token: server
                .base
                .join("di-oauth2-service/oauth/token")
                .expect("url"),
            api: server.base.clone(),
        };
        let file = TokenFile::new(dir.path().join("garmin_tokens.json"));
        file.save(&stored()).expect("seed tokens");
        let connect =
            GarminConnect::with_endpoints(file.clone(), endpoints, pacer).expect("client");
        (connect, server, file)
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 14).expect("valid date")
    }

    #[tokio::test]
    async fn open_refreshes_saves_the_rotated_tokens_and_reads_the_name() {
        let dir = TempDir::new().expect("tempdir");
        let replies = vec![Reply::new(200, DI_TOKEN), Reply::new(200, PROFILE)];
        let (connect, server, file) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        assert_eq!(session.display_name, "Jane Doe");
        let saved = file.load().expect("load");
        assert_eq!(saved.di_token, "new-access-token");
        assert_eq!(saved.di_refresh_token, "new-refresh-token");
        assert_eq!(saved.di_client_id, "client");
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        let refresh = &requests[0];
        assert_eq!(
            refresh.request_line(),
            "POST /di-oauth2-service/oauth/token HTTP/1.1"
        );
        assert!(refresh.has_header("authorization: Basic Y2xpZW50Og=="));
        assert!(refresh.has_header("user-agent: GCM-Android-5.23"));
        assert!(refresh.has_header("content-type: application/x-www-form-urlencoded"));
        assert_eq!(
            refresh.body,
            "grant_type=refresh_token&client_id=client&refresh_token=old-refresh"
        );
        let profile = &requests[1];
        assert_eq!(
            profile.request_line(),
            "GET /userprofile-service/socialProfile HTTP/1.1"
        );
        assert!(profile.has_header("authorization: Bearer new-access-token"));
    }

    #[tokio::test]
    async fn concurrent_opens_on_one_thread_both_complete() {
        let dir = TempDir::new().expect("tempdir");
        let both = r#"{"access_token":"a","refresh_token":"r","displayName":"Jane"}"#;
        let replies = (0..4).map(|_| Reply::new(200, both)).collect();
        let (connect, _server, _file) = connect(replies, &dir).await;
        let (first, second) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(connect.open(), connect.open())
        })
        .await
        .expect("no deadlock");
        assert!(first.is_ok() && second.is_ok());
    }

    #[tokio::test]
    async fn a_rejected_refresh_is_a_session_error_and_keeps_the_file() {
        let dir = TempDir::new().expect("tempdir");
        let (connect, server, file) = connect(vec![Reply::status(400)], &dir).await;
        let before = fs::read(dir.path().join("garmin_tokens.json")).expect("read");
        let error = connect.open().await.expect_err("refresh fails");
        assert!(matches!(error, GarminError::Refresh(status) if status.as_u16() == 400));
        assert!(GarminConnect::is_session_error(&error));
        assert_eq!(file.load().expect("load"), stored());
        assert_eq!(
            fs::read(dir.path().join("garmin_tokens.json")).expect("read"),
            before
        );
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_blank_refresh_token_keeps_the_stored_one() {
        let dir = TempDir::new().expect("tempdir");
        let body = r#"{"access_token":"new-access","refresh_token":"  "}"#;
        let replies = vec![Reply::new(200, body), Reply::new(200, PROFILE)];
        let (connect, _server, file) = connect(replies, &dir).await;
        connect.open().await.expect("session");
        let saved = file.load().expect("load");
        assert_eq!(saved.di_token, "new-access");
        assert_eq!(saved.di_refresh_token, "old-refresh");
    }

    #[tokio::test]
    async fn a_blank_access_token_is_rejected_before_saving() {
        let dir = TempDir::new().expect("tempdir");
        let body = r#"{"access_token":"","refresh_token":"new-refresh"}"#;
        let (connect, server, file) = connect(vec![Reply::new(200, body)], &dir).await;
        let error = connect.open().await.expect_err("blank access token");
        assert!(matches!(error, GarminError::Malformed));
        assert_eq!(file.load().expect("load"), stored());
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn open_without_a_token_file_sends_nothing() {
        let dir = TempDir::new().expect("tempdir");
        let (connect, server, _) = connect(vec![], &dir).await;
        fs::remove_file(dir.path().join("garmin_tokens.json")).expect("remove");
        let error = connect.open().await.expect_err("no tokens");
        assert!(matches!(error, GarminError::TokenFile { .. }));
        assert!(GarminConnect::is_session_error(&error));
        assert!(server.requests().is_empty());
    }

    #[test]
    fn an_unreadable_token_file_is_not_a_session_error() {
        let error = GarminError::TokenFile {
            path: "garmin_tokens.json".into(),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert!(!GarminConnect::is_session_error(&error));
    }

    #[tokio::test]
    async fn an_invalid_token_file_is_a_session_error() {
        let dir = TempDir::new().expect("tempdir");
        let (connect, server, _) = connect(vec![], &dir).await;
        fs::write(dir.path().join("garmin_tokens.json"), "not json").expect("write");
        let error = connect.open().await.expect_err("invalid tokens");
        assert!(matches!(error, GarminError::InvalidTokenFile { .. }));
        assert!(GarminConnect::is_session_error(&error));
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn only_explicit_refresh_refusals_are_session_errors() {
        for (status, session) in [
            (400, true),
            (401, true),
            (403, true),
            (408, false),
            (500, false),
            (427, false),
            (429, false),
        ] {
            let dir = TempDir::new().expect("tempdir");
            let (connect, _server, _file) = connect(vec![Reply::status(status)], &dir).await;
            let error = connect.open().await.expect_err("refresh fails");
            assert_eq!(GarminConnect::is_session_error(&error), session, "{status}");
            assert_eq!(
                GarminConnect::is_throttled(&error),
                matches!(status, 427 | 429),
                "{status}"
            );
        }
    }

    #[tokio::test]
    async fn daily_reads_the_summary_then_the_sleep_data() {
        let dir = TempDir::new().expect("tempdir");
        let replies = vec![
            Reply::new(200, DI_TOKEN),
            Reply::new(200, PROFILE),
            Reply::new(200, SUMMARY),
            Reply::new(200, SLEEP),
        ];
        let (connect, server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        let health = connect.daily(&session, date()).await.expect("daily");
        assert_eq!(health.steps, Some(8431));
        assert_eq!(health.resting_heart_rate, Some(52));
        assert_eq!(health.body_battery_max, Some(96));
        assert_eq!(health.body_battery_min, Some(14));
        assert_eq!(health.sleep_seconds, Some(27_120));
        assert_eq!(health.sleep_score, Some(81));
        let requests = server.requests();
        assert_eq!(
            requests[2].request_line(),
            "GET /usersummary-service/usersummary/daily/Jane%20Doe?calendarDate=2026-07-14 HTTP/1.1"
        );
        assert_eq!(
            requests[3].request_line(),
            "GET /wellness-service/wellness/dailySleepData/Jane%20Doe?date=2026-07-14&nonSleepBufferMinutes=60 HTTP/1.1"
        );
        for request in &requests[2..] {
            assert!(request.has_header("authorization: Bearer new-access-token"));
            assert!(request.has_header("x-garmin-client-platform: Android"));
        }
    }

    #[tokio::test]
    async fn a_refused_data_call_is_a_session_error_and_stops_the_day() {
        for status in [401, 403] {
            let dir = TempDir::new().expect("tempdir");
            let replies = vec![
                Reply::new(200, DI_TOKEN),
                Reply::new(200, PROFILE),
                Reply::status(status),
            ];
            let (connect, server, _) = connect(replies, &dir).await;
            let session = connect.open().await.expect("session");
            let error = connect.daily(&session, date()).await.expect_err("refused");
            assert!(matches!(error, GarminError::SessionRejected(_)));
            assert!(GarminConnect::is_session_error(&error));
            assert_eq!(server.requests().len(), 3);
        }
    }

    #[tokio::test]
    async fn other_data_failures_are_not_session_errors() {
        let dir = TempDir::new().expect("tempdir");
        let replies = vec![
            Reply::new(200, DI_TOKEN),
            Reply::new(200, PROFILE),
            Reply::status(500),
            Reply::new(200, SUMMARY),
            Reply::new(200, "not json"),
        ];
        let (connect, _server, _file) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        let status = connect.daily(&session, date()).await.expect_err("500");
        assert!(matches!(status, GarminError::Status(_)));
        let garbage = connect.daily(&session, date()).await.expect_err("garbage");
        assert!(matches!(garbage, GarminError::InvalidResponse { .. }));
        assert!(!GarminConnect::is_session_error(&status));
        assert!(!GarminConnect::is_session_error(&garbage));
        assert!(!GarminConnect::is_throttled(&status));
        assert!(!GarminConnect::is_throttled(&garbage));
    }

    #[tokio::test]
    async fn a_rejected_session_is_not_throttling() {
        let dir = TempDir::new().expect("tempdir");
        let replies = vec![
            Reply::new(200, DI_TOKEN),
            Reply::new(200, PROFILE),
            Reply::status(403),
        ];
        let (connect, _server, _file) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        let error = connect.daily(&session, date()).await.expect_err("refused");
        assert!(!GarminConnect::is_throttled(&error));
    }

    #[tokio::test]
    async fn throttling_on_a_daily_endpoint_is_throttled_but_not_a_session_error() {
        for status in [427, 429] {
            for sleep_call in [false, true] {
                let dir = TempDir::new().expect("tempdir");
                let mut replies = vec![Reply::new(200, DI_TOKEN), Reply::new(200, PROFILE)];
                if sleep_call {
                    replies.push(Reply::new(200, SUMMARY));
                }
                replies.push(Reply::status(status));
                let (connect, _server, _file) = connect(replies, &dir).await;
                let session = connect.open().await.expect("session");
                let error = connect
                    .daily(&session, date())
                    .await
                    .expect_err("throttled");
                assert!(matches!(error, GarminError::Status(_)));
                assert!(GarminConnect::is_throttled(&error));
                assert!(!GarminConnect::is_session_error(&error));
            }
        }
    }

    #[tokio::test]
    async fn nothing_secret_reaches_errors_or_debug_output() {
        let dir = TempDir::new().expect("tempdir");
        let replies = vec![
            Reply::new(200, DI_TOKEN),
            Reply::new(200, PROFILE),
            Reply::new(401, "body-with-secret-material"),
        ];
        let (connect, server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        let error = connect.daily(&session, date()).await.expect_err("refused");
        let rendered = format!("{error} {error:?} {session:?}");
        for secret in [
            "new-access-token",
            "new-refresh-token",
            "body-with-secret-material",
            "Jane",
            server.base.as_str(),
        ] {
            assert!(!rendered.contains(secret), "{secret} leaked");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_redirect_is_followed_by_hand_and_each_hop_is_paced() {
        static ONE_SECOND: Pacer = Pacer::new(Duration::from_secs(1)..=Duration::from_secs(1));
        let dir = TempDir::new().expect("tempdir");
        let replies = vec![
            Reply::new(200, DI_TOKEN),
            Reply::redirect("/moved/profile"),
            Reply::new(200, PROFILE),
        ];
        let (connect, server, _) = connect_paced(replies, &dir, &ONE_SECOND).await;
        let start = tokio::time::Instant::now();
        let session = connect.open().await.expect("session");
        assert_eq!(session.display_name, "Jane Doe");
        assert!(start.elapsed() >= Duration::from_secs(2));
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].request_line(), "GET /moved/profile HTTP/1.1");
        assert!(requests[2].has_header("authorization: Bearer new-access-token"));
    }

    #[tokio::test]
    async fn a_redirect_to_another_host_does_not_carry_the_token() {
        let dir = TempDir::new().expect("tempdir");
        let elsewhere = TestServer::start(vec![Reply::new(200, PROFILE)]).await;
        let target = format!("{}elsewhere", elsewhere.base);
        let replies = vec![Reply::new(200, DI_TOKEN), Reply::redirect(&target)];
        let (connect, _server, _) = connect(replies, &dir).await;
        let session = connect.open().await.expect("session");
        assert_eq!(session.display_name, "Jane Doe");
        let requests = elsewhere.requests();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].has_header("authorization"));
    }

    #[tokio::test]
    async fn more_than_three_redirects_end_as_a_status_error() {
        let dir = TempDir::new().expect("tempdir");
        let mut replies = vec![Reply::new(200, DI_TOKEN)];
        replies.extend((0..5).map(|_| Reply::redirect("/again")));
        let (connect, server, _) = connect(replies, &dir).await;
        let error = connect.open().await.expect_err("too many redirects");
        assert!(matches!(error, GarminError::Status(status) if status.as_u16() == 302));
        assert_eq!(server.requests().len(), 5);
    }

    #[test]
    fn every_client_shares_the_process_wide_pacer() {
        let dir = TempDir::new().expect("tempdir");
        let new = || GarminConnect::new(TokenFile::new(dir.path().join("t.json"))).expect("client");
        assert!(std::ptr::eq(new().pacer, new().pacer));
    }

    #[tokio::test]
    async fn a_transport_error_carries_no_url() {
        let dir = TempDir::new().expect("tempdir");
        let (connect, server, _) = connect(vec![], &dir).await;
        let base = server.base.to_string();
        drop(server);
        let error = connect.open().await.expect_err("server is gone");
        assert!(matches!(error, GarminError::Request(_)));
        assert!(!format!("{error:?}").contains(&base));
    }
}
