use reqwest::header::{ACCEPT_LANGUAGE, HeaderMap, HeaderName, HeaderValue, USER_AGENT};

/// The headers of Garmin's Android app, sent on token and API calls.
pub fn native_headers() -> HeaderMap {
    HeaderMap::from_iter([
        (USER_AGENT, HeaderValue::from_static("GCM-Android-5.23")),
        (
            HeaderName::from_static("x-garmin-user-agent"),
            HeaderValue::from_static(
                "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0",
            ),
        ),
        (
            HeaderName::from_static("x-garmin-paired-app-version"),
            HeaderValue::from_static("10861"),
        ),
        (
            HeaderName::from_static("x-garmin-client-platform"),
            HeaderValue::from_static("Android"),
        ),
        (
            HeaderName::from_static("x-app-ver"),
            HeaderValue::from_static("10861"),
        ),
        (
            HeaderName::from_static("x-lang"),
            HeaderValue::from_static("en"),
        ),
        (
            HeaderName::from_static("x-gcexperience"),
            HeaderValue::from_static("GC5"),
        ),
        (ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9")),
    ])
}
