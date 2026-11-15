use std::io;

use reqwest::StatusCode;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, COOKIE, HeaderMap, HeaderValue, ORIGIN, REFERER,
    USER_AGENT,
};
use serde_json::{Value, json};

use crate::cookie_jar::CookieJar;
use crate::endpoints::build_url;
use crate::garmin_connect::blocking;
use crate::native_headers::native_headers;
use crate::sso_response::SsoOutcome;
use crate::token_response::TokenResponse;
use crate::{GarminConnect, GarminError, Tokens};

const SSO_CLIENT_ID: &str = "GCM_IOS_DARK";
const SERVICE_URL: &str = "https://mobile.integration.garmin.com/gcm/ios";
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
const DI_GRANT_TYPE: &str =
    "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket";
const DI_CLIENT_IDS: [&str; 4] = [
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI",
    "GARMIN_CONNECT_MOBILE_IOS_DI",
];
const HTML_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const JSON_ACCEPT: &str = "application/json, text/plain, */*";
const EXCHANGE_ACCEPT: &str = "application/json,text/html;q=0.9,*/*;q=0.8";

impl GarminConnect {
    /// Signs in once, with an MFA step when Garmin asks for one, and saves the
    /// tokens. It never retries, and stops at the first 403, 427 or 429.
    pub async fn login(
        &self,
        email: &str,
        password: &str,
        mfa_code: impl FnOnce() -> io::Result<String>,
    ) -> Result<(), GarminError> {
        let mut jar = CookieJar::default();
        self.load_sign_in_page(&mut jar).await?;
        self.pause().await;
        let credentials = json!({
            "username": email,
            "password": password,
            "rememberMe": true,
            "captchaToken": "",
        });
        let outcome = self
            .post_sso(&mut jar, &["mobile", "api", "login"], &credentials)
            .await?;
        let ticket = match outcome {
            SsoOutcome::Ticket(ticket) => ticket,
            SsoOutcome::MfaRequired { method } => {
                self.verify_mfa(&mut jar, &method, mfa_code).await?
            }
        };
        let tokens = self.exchange(&ticket).await?;
        self.display_name(&tokens).await?;
        let file = self.token_file.clone();
        blocking(move || {
            let _lock = file.lock()?;
            file.save(&tokens)
        })
        .await
    }

    fn sign_in_url(&self) -> reqwest::Url {
        build_url(
            &self.endpoints.sso,
            &["mobile", "sso", "en_US", "sign-in"],
            &[("clientId", SSO_CLIENT_ID), ("service", SERVICE_URL)],
        )
    }

    async fn load_sign_in_page(&self, jar: &mut CookieJar) -> Result<(), GarminError> {
        let response = self
            .http
            .get(self.sign_in_url())
            .headers(browser_headers(HTML_ACCEPT))
            .send()
            .await
            .map_err(GarminError::request)?;
        jar.store(response.headers());
        let status = response.status();
        check_not_blocked(status)?;
        if status.is_success() {
            Ok(())
        } else {
            Err(GarminError::Status(status))
        }
    }

    async fn post_sso(
        &self,
        jar: &mut CookieJar,
        segments: &[&str],
        body: &Value,
    ) -> Result<SsoOutcome, GarminError> {
        let url = build_url(
            &self.endpoints.sso,
            segments,
            &[
                ("clientId", SSO_CLIENT_ID),
                ("locale", "en-US"),
                ("service", SERVICE_URL),
            ],
        );
        let mut request = self
            .http
            .post(url)
            .headers(browser_headers(JSON_ACCEPT))
            .header(ORIGIN, self.endpoints.sso.origin().ascii_serialization())
            .header(REFERER, self.sign_in_url().as_str())
            .json(body);
        if let Some(cookie) = jar.header() {
            request = request.header(COOKIE, cookie);
        }
        let response = request.send().await.map_err(GarminError::request)?;
        jar.store(response.headers());
        check_not_blocked(response.status())?;
        let bytes = response.bytes().await.map_err(GarminError::request)?;
        SsoOutcome::parse(&bytes)
    }

    async fn verify_mfa(
        &self,
        jar: &mut CookieJar,
        method: &str,
        mfa_code: impl FnOnce() -> io::Result<String>,
    ) -> Result<String, GarminError> {
        let code = mfa_code().map_err(GarminError::Prompt)?;
        self.pause().await;
        let body = json!({
            "mfaMethod": method,
            "mfaVerificationCode": code,
            "rememberMyBrowser": true,
            "reconsentList": [],
            "mfaSetup": false,
        });
        let outcome = self
            .post_sso(jar, &["mobile", "api", "mfa", "verifyCode"], &body)
            .await?;
        match outcome {
            SsoOutcome::Ticket(ticket) => Ok(ticket),
            SsoOutcome::MfaRequired { .. } => {
                Err(GarminError::Unexpected("MFA_REQUIRED".to_owned()))
            }
        }
    }

    /// Trades the service ticket for DI tokens, trying each client id in turn.
    async fn exchange(&self, ticket: &str) -> Result<Tokens, GarminError> {
        for client_id in DI_CLIENT_IDS {
            self.pause().await;
            let response = self
                .http
                .post(self.endpoints.token.clone())
                .headers(native_headers())
                .basic_auth(client_id, Some(""))
                .header(ACCEPT, EXCHANGE_ACCEPT)
                .header(CACHE_CONTROL, "no-cache")
                .form(&[
                    ("client_id", client_id),
                    ("service_ticket", ticket),
                    ("grant_type", DI_GRANT_TYPE),
                    ("service_url", SERVICE_URL),
                ])
                .send()
                .await
                .map_err(GarminError::request)?;
            let status = response.status();
            check_not_blocked(status)?;
            if status.is_success() {
                let body: TokenResponse = response.json().await.map_err(GarminError::request)?;
                return body.into_tokens(client_id.to_owned(), None);
            }
        }
        Err(GarminError::Exchange)
    }
}

fn browser_headers(accept: &'static str) -> HeaderMap {
    HeaderMap::from_iter([
        (USER_AGENT, HeaderValue::from_static(LOGIN_USER_AGENT)),
        (ACCEPT, HeaderValue::from_static(accept)),
        (ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9")),
    ])
}

/// Garmin answers 403 (challenge), 427 or 429 (rate limit) when it blocks a
/// client; going on would only deepen the block.
fn check_not_blocked(status: StatusCode) -> Result<(), GarminError> {
    match status.as_u16() {
        403 | 427 | 429 => Err(GarminError::Blocked(status)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;
    use std::time::Duration;

    use tempfile::TempDir;
    use use_cases::HealthProvider;

    use crate::test_server::{Reply, TestServer};
    use crate::{Endpoints, GarminConnect, GarminError, TokenFile};

    const PASSWORD: &str = "hunter2-pw";
    const SUCCESS: &str = include_str!("../fixtures/sso_success.json");
    const MFA_REQUIRED: &str = include_str!("../fixtures/sso_mfa_required.json");
    const MFA_SUCCESS: &str = include_str!("../fixtures/sso_mfa_success.json");
    const INVALID: &str = include_str!("../fixtures/sso_invalid.json");
    const DI_TOKEN: &str = include_str!("../fixtures/di_token.json");
    const PROFILE: &str = include_str!("../fixtures/social_profile.json");

    struct Run {
        result: Result<(), GarminError>,
        server: TestServer,
        file: TokenFile,
        mfa_calls: u32,
        _dir: TempDir,
    }

    impl Run {
        fn saved(&self) -> bool {
            self.file.load().is_ok()
        }
    }

    fn sign_in_page() -> Reply {
        Reply::new(200, "<html></html>").with_cookie("session=abc; Path=/; HttpOnly")
    }

    async fn run_with(
        replies: Vec<Reply>,
        mfa_code: impl FnOnce() -> io::Result<String>,
    ) -> (Result<(), GarminError>, TestServer, TokenFile, TempDir) {
        let dir = TempDir::new().expect("tempdir");
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
        let connect =
            GarminConnect::with_endpoints(file.clone(), endpoints, Duration::ZERO).expect("client");
        let result = connect.login("jane@example.test", PASSWORD, mfa_code).await;
        (result, server, file, dir)
    }

    async fn run(replies: Vec<Reply>) -> Run {
        let calls = Cell::new(0_u32);
        let (result, server, file, dir) = run_with(replies, || {
            calls.set(calls.get() + 1);
            Ok("123456".to_owned())
        })
        .await;
        Run {
            result,
            server,
            file,
            mfa_calls: calls.get(),
            _dir: dir,
        }
    }

    #[tokio::test]
    async fn sign_in_without_mfa_saves_the_tokens() {
        let replies = vec![
            sign_in_page(),
            Reply::new(200, SUCCESS),
            Reply::new(200, DI_TOKEN),
            Reply::new(200, PROFILE),
        ];
        let run = run(replies).await;
        run.result.as_ref().expect("login succeeds");
        assert_eq!(run.mfa_calls, 0);
        let saved = run.file.load().expect("tokens saved");
        assert_eq!(saved.di_token, "new-access-token");
        assert_eq!(saved.di_refresh_token, "new-refresh-token");
        assert_eq!(
            saved.di_client_id,
            "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
        );
        let requests = run.server.requests();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].request_line().starts_with(
            "GET /mobile/sso/en_US/sign-in?clientId=GCM_IOS_DARK&service=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fios "
        ));
        let login = &requests[1];
        assert!(
            login
                .request_line()
                .starts_with("POST /mobile/api/login?clientId=GCM_IOS_DARK&locale=en-US&service=")
        );
        assert!(login.has_header("cookie: session=abc"));
        assert!(login.has_header("content-type: application/json"));
        assert!(login.has_header("referer: http://127.0.0.1"));
        assert!(login.has_header("origin: http://127.0.0.1"));
        assert!(login.body.contains("\"username\":\"jane@example.test\""));
        assert!(login.body.contains("\"captchaToken\":\"\""));
        let exchange = &requests[2];
        assert!(exchange.body.contains("service_ticket=ST-TESTTICKET-sso"));
        assert!(
            exchange
                .body
                .contains("client_id=GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2")
        );
        assert!(exchange.has_header("authorization: Basic "));
        assert!(requests[3].has_header("authorization: Bearer new-access-token"));
    }

    #[tokio::test]
    async fn mfa_asks_for_the_code_once_and_posts_it() {
        let replies = vec![
            sign_in_page(),
            Reply::new(200, MFA_REQUIRED),
            Reply::new(200, MFA_SUCCESS),
            Reply::new(200, DI_TOKEN),
            Reply::new(200, PROFILE),
        ];
        let run = run(replies).await;
        run.result.as_ref().expect("login succeeds");
        assert_eq!(run.mfa_calls, 1);
        let requests = run.server.requests();
        let mfa = &requests[2];
        assert!(
            mfa.request_line()
                .starts_with("POST /mobile/api/mfa/verifyCode?")
        );
        assert!(mfa.has_header("cookie: session=abc"));
        assert!(mfa.body.contains("\"mfaVerificationCode\":\"123456\""));
        assert!(mfa.body.contains("\"mfaMethod\":\"email\""));
        assert!(requests[3].body.contains("service_ticket=ST-MFATICKET-sso"));
        assert!(run.saved());
    }

    #[tokio::test]
    async fn invalid_credentials_stop_before_the_exchange() {
        let replies = vec![sign_in_page(), Reply::new(401, INVALID)];
        let run = run(replies).await;
        assert!(matches!(run.result, Err(GarminError::InvalidCredentials)));
        assert_eq!(run.server.requests().len(), 2);
        assert!(!run.saved());
    }

    async fn assert_blocked(prefix: fn() -> Vec<Reply>) {
        for status in [403, 427, 429] {
            let mut replies = prefix();
            replies.push(Reply::status(status));
            let expected = replies.len();
            let run = run(replies).await;
            let blocked = matches!(
                &run.result,
                Err(GarminError::Blocked(blocked)) if blocked.as_u16() == status
            );
            assert!(blocked, "status {status} blocks");
            assert_eq!(run.server.requests().len(), expected, "status {status}");
            assert!(!run.saved());
        }
    }

    #[tokio::test]
    async fn blocked_on_the_sign_in_page_sends_nothing_more() {
        assert_blocked(Vec::new).await;
    }

    #[tokio::test]
    async fn blocked_on_the_login_post_sends_nothing_more() {
        assert_blocked(|| vec![sign_in_page()]).await;
    }

    #[tokio::test]
    async fn blocked_on_the_mfa_post_sends_nothing_more() {
        assert_blocked(|| vec![sign_in_page(), Reply::new(200, MFA_REQUIRED)]).await;
    }

    #[tokio::test]
    async fn blocked_on_the_first_exchange_sends_nothing_more() {
        assert_blocked(|| vec![sign_in_page(), Reply::new(200, SUCCESS)]).await;
    }

    #[tokio::test]
    async fn blocked_on_a_later_exchange_sends_nothing_more() {
        assert_blocked(|| vec![sign_in_page(), Reply::new(200, SUCCESS), Reply::status(500)]).await;
    }

    #[tokio::test]
    async fn a_rate_limit_in_the_json_body_is_a_block() {
        let body = r#"{"error":{"status-code":"429"}}"#;
        let run = run(vec![sign_in_page(), Reply::new(200, body)]).await;
        assert!(matches!(run.result, Err(GarminError::Blocked(_))));
        assert_eq!(run.server.requests().len(), 2);
    }

    #[tokio::test]
    async fn the_next_client_id_is_tried_after_a_failed_exchange() {
        let replies = vec![
            sign_in_page(),
            Reply::new(200, SUCCESS),
            Reply::status(401),
            Reply::new(200, DI_TOKEN),
            Reply::new(200, PROFILE),
        ];
        let run = run(replies).await;
        run.result.as_ref().expect("login succeeds");
        let requests = run.server.requests();
        assert!(
            requests[3]
                .body
                .contains("client_id=GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4")
        );
        assert_eq!(
            run.file.load().expect("saved").di_client_id,
            "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4"
        );
    }

    #[tokio::test]
    async fn every_client_id_failing_saves_nothing() {
        let replies = vec![
            sign_in_page(),
            Reply::new(200, SUCCESS),
            Reply::status(401),
            Reply::status(401),
            Reply::status(401),
            Reply::status(401),
        ];
        let run = run(replies).await;
        assert!(matches!(run.result, Err(GarminError::Exchange)));
        assert_eq!(run.server.requests().len(), 6);
        assert!(!run.saved());
    }

    #[tokio::test]
    async fn a_refused_profile_check_saves_nothing() {
        let replies = vec![
            sign_in_page(),
            Reply::new(200, SUCCESS),
            Reply::new(200, DI_TOKEN),
            Reply::status(401),
        ];
        let run = run(replies).await;
        let rejected = matches!(&run.result, Err(error) if GarminConnect::is_session_error(error));
        assert!(rejected);
        assert!(!run.saved());
    }

    #[tokio::test]
    async fn a_failed_prompt_stops_the_login() {
        let replies = vec![sign_in_page(), Reply::new(200, MFA_REQUIRED)];
        let (result, server, file, _dir) =
            run_with(replies, || Err(io::Error::other("no terminal"))).await;
        assert!(matches!(result, Err(GarminError::Prompt(_))));
        assert_eq!(server.requests().len(), 2);
        assert!(file.load().is_err());
    }

    #[tokio::test]
    async fn no_secret_reaches_an_error() {
        let scripts = vec![
            vec![sign_in_page(), Reply::new(401, INVALID)],
            vec![
                sign_in_page(),
                Reply::new(200, r#"{"responseStatus":{"type":"LOCKED"}}"#),
            ],
            vec![
                sign_in_page(),
                Reply::new(200, "<html>ST-TESTTICKET-sso</html>"),
            ],
            vec![
                sign_in_page(),
                Reply::new(200, SUCCESS),
                Reply::status(401),
                Reply::status(401),
                Reply::status(401),
                Reply::status(401),
            ],
            vec![Reply::status(429)],
        ];
        for replies in scripts {
            let run = run(replies).await;
            let error = run.result.expect_err("login fails");
            let rendered = format!("{error} {error:?}");
            for secret in [
                PASSWORD,
                "ST-TESTTICKET-sso",
                "jane@example.test",
                "session=abc",
            ] {
                assert!(!rendered.contains(secret), "{secret} leaked");
            }
        }
    }
}
