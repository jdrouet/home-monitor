use std::collections::BTreeMap;

use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};

/// Cookies for the single SSO host, kept by name. Expiry and attributes are
/// ignored: the login lasts a few requests.
#[derive(Default)]
pub struct CookieJar {
    cookies: BTreeMap<String, String>,
}

impl CookieJar {
    pub fn store(&mut self, headers: &HeaderMap) {
        for value in headers.get_all(SET_COOKIE) {
            let pair = value
                .to_str()
                .ok()
                .and_then(|text| text.split(';').next())
                .and_then(|pair| pair.split_once('='));
            if let Some((name, value)) = pair {
                self.cookies
                    .insert(name.trim().to_owned(), value.trim().to_owned());
            }
        }
    }

    /// The `Cookie` header value, or `None` while the jar is empty.
    pub fn header(&self) -> Option<HeaderValue> {
        let joined = self
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        HeaderValue::from_str(&joined)
            .ok()
            .filter(|v| !v.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};

    use super::CookieJar;

    fn set_cookies(values: &[&'static str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(SET_COOKIE, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn an_empty_jar_has_no_header() {
        assert!(CookieJar::default().header().is_none());
    }

    #[test]
    fn cookies_are_joined_and_attributes_dropped() {
        let mut jar = CookieJar::default();
        jar.store(&set_cookies(&["a=1; Path=/; HttpOnly", "b=2"]));
        assert_eq!(jar.header().expect("header"), "a=1; b=2");
    }

    #[test]
    fn a_later_cookie_replaces_an_earlier_one_by_name() {
        let mut jar = CookieJar::default();
        jar.store(&set_cookies(&["a=1"]));
        jar.store(&set_cookies(&["a=2", "malformed"]));
        assert_eq!(jar.header().expect("header"), "a=2");
    }
}
