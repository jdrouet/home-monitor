use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use tracing::error;

pub const APP_CSS: &str = include_str!("../assets/app.css");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    History,
    Alerts,
    Health,
}

impl Page {
    /// Every page, in navigation order.
    pub const ALL: [Page; 4] = [Page::Overview, Page::History, Page::Alerts, Page::Health];

    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::History => "History",
            Page::Alerts => "Alerts",
            Page::Health => "Health",
        }
    }

    pub fn href(self) -> &'static str {
        match self {
            Page::Overview => "/",
            Page::History => "/history",
            Page::Alerts => "/alerts",
            Page::Health => "/health",
        }
    }

    /// Seconds between reloads, or `None` for a page that never reloads.
    pub fn refresh_secs(self) -> Option<u32> {
        match self {
            Page::Overview | Page::Alerts => Some(60),
            Page::Health => Some(300),
            Page::History => None,
        }
    }
}

/// Renders `page` with `status`, or answers 500 when the template fails.
pub fn respond<T: Template>(page: &T, status: StatusCode) -> Response {
    match page.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(error) => {
            error!(%error, page = std::any::type_name::<T>(), "cannot render page");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Asserts the shell: one current link, the skip link, the nav, the title and no meta refresh.
#[cfg(test)]
pub fn assert_shell(html: &str, current: Page) {
    assert_eq!(html.matches(r#" aria-current="page">"#).count(), 1);
    assert!(html.contains(&format!(
        r#"<a href="{}" aria-current="page">{}</a>"#,
        current.href(),
        current.label()
    )));
    for page in Page::ALL {
        assert!(html.contains(&format!(r#"href="{}""#, page.href())));
    }
    let skip = html.find("Skip to content").expect("skip link");
    let nav = html.find(r#"<nav aria-label="Main">"#).expect("nav");
    let main = html.find(r#"<main id="main">"#).expect("main");
    assert!(skip < nav && nav < main);
    assert!(html.contains(&format!(
        "<title>{} · Home monitor</title>",
        current.label()
    )));
    assert!(!html.contains(r#"http-equiv="refresh""#));
}

#[cfg(test)]
mod tests {
    use super::Page;

    #[test]
    fn pages_are_listed_in_nav_order_with_labels_hrefs_and_refresh_periods() {
        let rows: Vec<_> = Page::ALL
            .iter()
            .map(|page| (page.label(), page.href(), page.refresh_secs()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Overview", "/", Some(60)),
                ("History", "/history", None),
                ("Alerts", "/alerts", Some(60)),
                ("Health", "/health", Some(300)),
            ]
        );
    }
}
