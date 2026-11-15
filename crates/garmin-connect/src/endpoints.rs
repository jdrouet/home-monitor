use reqwest::Url;

const SSO: &str = "https://sso.garmin.com";
const TOKEN: &str = "https://diauth.garmin.com/di-oauth2-service/oauth/token";
const API: &str = "https://connectapi.garmin.com";

/// The hosts the adapter talks to. Tests point them at a local server.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub sso: Url,
    pub token: Url,
    pub api: Url,
}

impl Endpoints {
    pub fn garmin() -> Self {
        Self {
            sso: parse(SSO),
            token: parse(TOKEN),
            api: parse(API),
        }
    }
}

fn parse(text: &str) -> Url {
    Url::parse(text).expect("constant URL is valid")
}

/// `base` with `segments` appended, each percent-encoded as one path segment,
/// and `query` appended.
pub fn build_url(base: &Url, segments: &[&str], query: &[(&str, &str)]) -> Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("endpoint is a base URL")
        .pop_if_empty()
        .extend(segments);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    url
}

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::{Endpoints, build_url};

    #[test]
    fn garmin_endpoints_are_https() {
        let endpoints = Endpoints::garmin();
        for url in [endpoints.sso, endpoints.token, endpoints.api] {
            assert_eq!(url.scheme(), "https");
        }
    }

    #[test]
    fn build_url_encodes_each_segment_and_the_query() {
        let base = Url::parse("https://example.test").expect("url");
        let url = build_url(&base, &["a", "b/c d"], &[("date", "2026-07-14")]);
        assert_eq!(
            url.as_str(),
            "https://example.test/a/b%2Fc%20d?date=2026-07-14"
        );
    }

    #[test]
    fn build_url_without_a_query_has_no_question_mark() {
        let base = Url::parse("https://example.test").expect("url");
        assert_eq!(
            build_url(&base, &["a"], &[]).as_str(),
            "https://example.test/a"
        );
    }
}
