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

/// The sign-in link the system browser may open for one helper: https, no user name, no control
/// characters, and a host that is one of the helper's own sites or a subdomain of one.
pub fn signin_target(helper: &str, link: &str) -> Option<String> {
    if link
        .chars()
        .any(|c| c == '\\' || c.is_control() || c.is_whitespace())
    {
        return None;
    }
    let sites = crate::helpers::find_spec(helper)?.sites;
    let url = Url::parse(link).ok()?;
    let host = url.host_str()?;
    let own = sites
        .iter()
        .any(|s| host == *s || host.strip_suffix(s).is_some_and(|h| h.ends_with('.')));
    (url.scheme() == "https" && url.username().is_empty() && url.password().is_none() && own)
        .then(|| String::from(url))
}

/// The title of the control window.
pub const CONTROL_TITLE: &str = "HQ Hub";

/// The title of the window that shows HQ's web app: plain "HQ", or with a note that it is the
/// HQ on the person's phone when a paired hub replaces the one on this computer.
pub fn main_title(hub_web: bool) -> String {
    if hub_web {
        crate::words::word("hub_window_phone")
    } else {
        "HQ".to_string()
    }
}

/// The page of HQ's web app a notice or a card may open, or none. Only these are allowed:
/// `/`, `/chat`, `/notes`, `/inbox`, `/helpers`, `/settings/devices`, `/helpers/<id>` and
/// `/inbox/<id>`, with no query, no fragment and an id of letters, digits, `-` and `_`. Anything
/// else (a full address, `..`, a backslash) is refused, so a card can never steer the window
/// elsewhere.
pub fn main_path_allowed(path: &str) -> Option<String> {
    let id_ok = |id: &str| {
        (1..=64).contains(&id.len())
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    let allowed = match path {
        "/" | "/chat" | "/notes" | "/inbox" | "/helpers" | "/settings/devices" => true,
        _ => path
            .strip_prefix("/helpers/")
            .or_else(|| path.strip_prefix("/inbox/"))
            .is_some_and(id_ok),
    };
    allowed.then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "http://127.0.0.1:47474";

    #[test]
    fn a_sign_in_link_opens_only_on_the_helpers_own_site() {
        let ok = |h, l| signin_target(h, l).is_some();
        assert!(ok("claude-code", "https://claude.ai/login?x=1"));
        assert!(ok("claude-code", "https://console.anthropic.com/oauth"));
        assert!(!ok("claude-code", "http://claude.ai/login"));
        assert!(!ok("claude-code", "https://claude.ai.evil.example/"));
        assert!(!ok("claude-code", "https://evilclaude.ai/"));
        assert!(!ok("claude-code", "https://claude.ai@evil.example/"));
        assert!(!ok("claude-code", "https://auth.openai.com/"));
        assert!(!ok("pi", "https://claude.ai/"));
        assert!(!ok("nobody", "https://claude.ai/"));
    }

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

    #[test]
    fn the_windows_are_titled_for_what_they_show() {
        assert_eq!(CONTROL_TITLE, "HQ Hub");
        assert_eq!(main_title(false), "HQ");
        assert_eq!(main_title(true), "HQ · HQ on your phone");
    }

    #[test]
    fn the_window_opens_only_the_pages_a_card_may_name() {
        for ok in [
            "/",
            "/chat",
            "/notes",
            "/inbox",
            "/helpers",
            "/settings/devices",
            "/helpers/abc_12-X",
            "/inbox/9f3",
        ] {
            assert_eq!(main_path_allowed(ok).as_deref(), Some(ok));
        }
        for bad in [
            "",
            "https://example.com",
            "//example.com",
            "/helpers/",
            "/helpers/..",
            "/helpers/a/b",
            "/helpers/a?x=1",
            "/helpers/a#t",
            "/inbox/a\\b",
            "/settings",
            "/helpers/%2e%2e",
            "chat",
        ] {
            assert_eq!(main_path_allowed(bad), None, "{bad}");
        }
        assert_eq!(
            main_path_allowed(&format!("/helpers/{}", "a".repeat(65))),
            None
        );
    }
}
