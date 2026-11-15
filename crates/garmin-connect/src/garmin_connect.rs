use std::time::Duration;

use chrono::NaiveDate;
use reqwest::header::{ACCEPT, CACHE_CONTROL};
use reqwest::{Client, StatusCode, Url};
use serde::de::DeserializeOwned;
use tokio::task::spawn_blocking;
use tokio::time::sleep;
use use_cases::{DailyHealth, HealthProvider};

use crate::daily_sleep::DailySleep;
use crate::daily_summary::DailySummary;
use crate::endpoints::build_url;
use crate::native_headers::native_headers;
use crate::social_profile::SocialProfile;
use crate::token_response::TokenResponse;
use crate::{Endpoints, GarminError, GarminSession, TokenFile, Tokens};

const PAUSE: Duration = Duration::from_secs(3);
const TIMEOUT: Duration = Duration::from_secs(30);

/// Garmin Connect's unofficial API, reached with the tokens in a token file.
pub struct GarminConnect {
    pub(crate) http: Client,
    pub(crate) endpoints: Endpoints,
    pub(crate) token_file: TokenFile,
    pub(crate) pause: Duration,
}

impl GarminConnect {
    pub fn new(token_file: TokenFile) -> Result<Self, GarminError> {
        Self::with_endpoints(token_file, Endpoints::garmin(), PAUSE)
    }

    pub(crate) fn with_endpoints(
        token_file: TokenFile,
        endpoints: Endpoints,
        pause: Duration,
    ) -> Result<Self, GarminError> {
        let http = Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(GarminError::request)?;
        Ok(Self {
            http,
            endpoints,
            token_file,
            pause,
        })
    }

    pub(crate) async fn pause(&self) {
        sleep(self.pause).await;
    }

    /// Refreshes the stored tokens and saves the result before any other call.
    /// The lock is held from the load to the save and is taken off the runtime
    /// thread, which other lock holders' requests may need.
    async fn refresh_and_save(&self) -> Result<Tokens, GarminError> {
        let file = self.token_file.clone();
        let (lock, stored) = blocking(move || Ok((file.lock()?, file.load()?))).await?;
        let fresh = self.refresh(&stored).await?;
        let file = self.token_file.clone();
        let saved = blocking(move || file.save(&fresh).map(|()| fresh)).await;
        drop(lock);
        saved
    }

    async fn refresh(&self, stored: &Tokens) -> Result<Tokens, GarminError> {
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
        let body: TokenResponse = response.json().await.map_err(GarminError::request)?;
        body.into_tokens(stored.di_client_id.clone(), Some(&stored.di_refresh_token))
    }

    /// GETs `url` as the signed-in user. A 401 or 403 means the session is
    /// refused.
    async fn fetch<T: DeserializeOwned>(&self, token: &str, url: Url) -> Result<T, GarminError> {
        let response = self
            .http
            .get(url)
            .headers(native_headers())
            .header(ACCEPT, "application/json")
            .bearer_auth(token)
            .send()
            .await
            .map_err(GarminError::request)?;
        match response.status() {
            status @ (StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => {
                Err(GarminError::SessionRejected(status))
            }
            status if !status.is_success() => Err(GarminError::Status(status)),
            _ => response.json().await.map_err(GarminError::request),
        }
    }

    pub(crate) async fn display_name(&self, tokens: &Tokens) -> Result<String, GarminError> {
        let url = build_url(
            &self.endpoints.api,
            &["userprofile-service", "socialProfile"],
            &[],
        );
        let profile: SocialProfile = self.fetch(&tokens.di_token, url).await?;
        profile.display_name()
    }
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
        self.pause().await;
        let summary: DailySummary = self.fetch(token, summary_url).await?;
        self.pause().await;
        let sleep: DailySleep = self.fetch(token, sleep_url).await?;
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
        matches!(
            error,
            GarminError::Refresh(_) | GarminError::SessionRejected(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use chrono::NaiveDate;
    use tempfile::TempDir;
    use use_cases::HealthProvider;

    use crate::test_server::{Reply, TestServer};
    use crate::{Endpoints, GarminConnect, GarminError, TokenFile, Tokens};

    const DI_TOKEN: &str = include_str!("../fixtures/di_token.json");
    const PROFILE: &str = include_str!("../fixtures/social_profile.json");
    const SUMMARY: &str = include_str!("../fixtures/user_summary.json");
    const SLEEP: &str = include_str!("../fixtures/daily_sleep.json");

    fn stored() -> Tokens {
        Tokens {
            di_token: "old-access".to_owned(),
            di_refresh_token: "old-refresh".to_owned(),
            di_client_id: "client".to_owned(),
        }
    }

    async fn connect(replies: Vec<Reply>, dir: &TempDir) -> (GarminConnect, TestServer, TokenFile) {
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
            GarminConnect::with_endpoints(file.clone(), endpoints, Duration::ZERO).expect("client");
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
    async fn open_without_a_token_file_sends_nothing() {
        let dir = TempDir::new().expect("tempdir");
        let (connect, server, _) = connect(vec![], &dir).await;
        fs::remove_file(dir.path().join("garmin_tokens.json")).expect("remove");
        let error = connect.open().await.expect_err("no tokens");
        assert!(matches!(error, GarminError::TokenFile { .. }));
        assert!(!GarminConnect::is_session_error(&error));
        assert!(server.requests().is_empty());
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
        assert!(matches!(garbage, GarminError::Request(_)));
        assert!(!GarminConnect::is_session_error(&status));
        assert!(!GarminConnect::is_session_error(&garbage));
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
