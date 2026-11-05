//! A device's name from its `User-Agent`, by browser and system ("Firefox on Windows"), so the
//! owner never sees the raw header. Only the family is read; user-agent reduction keeps it.

/// Browsers by a marker in the header, checked in this order (Edge and Samsung Internet also
/// say Chrome, and Chrome also says Safari).
const BROWSERS: &[(&str, &str)] = &[
    ("Edg/", "Edge"),
    ("EdgiOS/", "Edge"),
    ("SamsungBrowser/", "Samsung Internet"),
    ("Firefox/", "Firefox"),
    ("FxiOS/", "Firefox"),
    ("CriOS/", "Chrome"),
    ("Chrome/", "Chrome"),
    ("Safari/", "Safari"),
    // The phone app's own HTTP client.
    ("Dalvik/", "HQ"),
    ("okhttp/", "HQ"),
];

/// Systems by a marker, in this order (an iPhone also says "like Mac OS X", Android says Linux).
const SYSTEMS: &[(&str, &str)] = &[
    ("iPhone", "iPhone"),
    ("iPad", "iPad"),
    ("Android", "Android"),
    ("Windows", "Windows"),
    ("Macintosh", "macOS"),
    ("Mac OS X", "macOS"),
    ("CrOS", "ChromeOS"),
    ("Linux", "Linux"),
];

fn first(ua: &str, table: &[(&str, &'static str)]) -> Option<&'static str> {
    table.iter().find(|(m, _)| ua.contains(m)).map(|(_, n)| *n)
}

/// The browser and the system a user agent names, each when it is one HQ knows.
pub(crate) fn parse(ua: &str) -> (Option<&'static str>, Option<&'static str>) {
    (first(ua, BROWSERS), first(ua, SYSTEMS))
}

/// "{Browser} on {System}", or `fallback` when the header names neither.
pub(crate) fn label(browser: Option<&str>, system: Option<&str>, fallback: &str) -> String {
    match (browser, system) {
        (Some(b), Some(s)) => format!("{b} on {s}"),
        (Some(b), None) => b.to_string(),
        (None, Some(s)) => format!("A browser on {s}"),
        (None, None) => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(ua: &str) -> String {
        let (b, s) = parse(ua);
        label(b, s, "Signed in with a code")
    }

    #[test]
    fn common_browsers_get_their_short_name() {
        assert_eq!(
            name(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:131.0) Gecko/20100101 Firefox/131.0"
            ),
            "Firefox on Windows"
        );
        assert_eq!(
            name(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
            ),
            "Chrome on macOS"
        );
        assert_eq!(
            name(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0"
            ),
            "Edge on Windows"
        );
        assert_eq!(
            name(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"
            ),
            "Safari on iPhone"
        );
        assert_eq!(
            name(
                "Mozilla/5.0 (Linux; Android 14; SM-A325F) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/26.0 Chrome/122.0.0.0 Mobile Safari/537.36"
            ),
            "Samsung Internet on Android"
        );
        assert_eq!(
            name("Dalvik/2.1.0 (Linux; U; Android 14; SM-A325F)"),
            "HQ on Android"
        );
        assert_eq!(name(""), "Signed in with a code");
    }
}
