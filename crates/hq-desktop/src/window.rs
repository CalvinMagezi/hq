//! Which addresses the app's windows may show, and which the app may hand to the system browser.
//! The shell asks these before every navigation, so a link inside HQ's pages cannot take a
//! window anywhere else.

use reqwest::Url;

/// The scheme and host Tauri serves the control window's own files from on macOS.
const APP_SCHEME: &str = "tauri";
const APP_HOST: &str = "localhost";

/// The control window may show its own bundled page and nothing else.
pub fn app_page_allowed(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| {
        u.scheme() == APP_SCHEME
            && u.host_str() == Some(APP_HOST)
            && u.port().is_none()
            && u.username().is_empty()
            && u.password().is_none()
    })
}

/// A window of HQ's web app may show pages of the proxy's own origin and nothing else. `origin`
/// is `http://127.0.0.1:<port>`; a lookalike (`http://127.0.0.1:474741`, `http://127.0.0.1:47474.evil.com`,
/// `https://…`, userinfo tricks) is another origin and is refused.
pub fn main_page_allowed(url: &str, origin: &str) -> bool {
    let (Ok(url), Ok(origin)) = (Url::parse(url), Url::parse(origin)) else {
        return false;
    };
    url.scheme() == "http"
        && url.scheme() == origin.scheme()
        && url.host_str() == origin.host_str()
        && url.port_or_known_default() == origin.port_or_known_default()
        && url.username().is_empty()
        && url.password().is_none()
}

/// The address to hand to the system browser, or `None`. It must be on the proxy's origin, hold
/// nothing a second parser could read differently (a backslash is a slash to one and part of the
/// user name to another), and goes out in the parser's own spelling.
pub fn browser_target(url: &str, origin: &str) -> Option<String> {
    if url
        .chars()
        .any(|c| c == '\\' || c.is_control() || c.is_whitespace())
    {
        return None;
    }
    if !main_page_allowed(url, origin) {
        return None;
    }
    Url::parse(url).ok().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "http://127.0.0.1:47474";

    #[test]
    fn the_control_window_stays_on_its_bundled_page() {
        assert!(app_page_allowed("tauri://localhost/"));
        assert!(app_page_allowed("tauri://localhost/index.html"));
        for bad in [
            "http://tauri.localhost/",
            "tauri://evil.example/",
            "tauri://localhost:8080/",
            "https://example.com/",
            "http://127.0.0.1:47474/",
            "file:///etc/passwd",
            "tauri://user@localhost/",
            "not a url",
        ] {
            assert!(!app_page_allowed(bad), "{bad}");
        }
    }

    #[test]
    fn only_a_plain_address_on_the_proxy_is_handed_to_the_browser() {
        assert_eq!(
            browser_target("http://127.0.0.1:47474/#code=ABCD", ORIGIN).as_deref(),
            Some("http://127.0.0.1:47474/#code=ABCD")
        );
        for bad in [
            "http://127.0.0.1:47474\\@evil.example/",
            "http://127.0.0.1:47474/ x",
            "http://127.0.0.1:47474/\nx",
            "http://evil.example/",
            "file:///etc/passwd",
        ] {
            assert_eq!(browser_target(bad, ORIGIN), None, "{bad:?}");
        }
    }

    #[test]
    fn the_web_window_stays_on_the_proxy_origin() {
        for ok in [
            "http://127.0.0.1:47474/",
            "http://127.0.0.1:47474/connect?request=abc",
            "http://127.0.0.1:47474/#token=t",
        ] {
            assert!(main_page_allowed(ok, ORIGIN), "{ok}");
        }
        for bad in [
            "http://127.0.0.1:474741/",
            "http://127.0.0.1:47474.evil.com/",
            "http://127.0.0.1:5678/",
            "http://localhost:47474/",
            "https://127.0.0.1:47474/",
            "http://user@127.0.0.1:47474/",
            "http://127.0.0.1:47474@evil.example/",
            "http://evil.example/",
            "tauri://localhost/",
            "javascript:alert(1)",
            "",
        ] {
            assert!(!main_page_allowed(bad, ORIGIN), "{bad}");
        }
    }
}
