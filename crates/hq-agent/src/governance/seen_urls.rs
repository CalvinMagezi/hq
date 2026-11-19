//! URLs HQ saw arrive from outside the model. A `web_fetch` of exactly one of them carries no
//! bytes the model wrote, so it needs no approval; a URL the model composed on a host nobody
//! showed it can carry notes in its path, query or even its host name, so it is refused.
//!
//! Matching is exact on a normalized form (lowercase scheme and host, default port and fragment
//! dropped, path and query untouched). A prefix match would let `https://x.example/?d=<notes>`
//! pass after `https://x.example/` was seen.

use regex::Regex;
use std::sync::LazyLock;

/// URLs one tracker remembers; the persisted set is capped separately per thread.
pub const MAX_SEEN: usize = 2000;

static URL_IN_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https?://[^\s<>"'`\\]+"#).expect("static url pattern must compile")
});

const TRAILING_PUNCTUATION: &[char] = &['.', ',', ';', ':', '!', '?', ']', '}', '*', '_'];

/// The form two URLs are compared in, or `None` for one that is never trusted: not http(s), with
/// a user name or password, or without a host.
pub fn normalize(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none_or(str::is_empty)
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

/// Every URL in `text`, normalized. A URL that ends in `)` is also listed without it, so a
/// Markdown link and a Wikipedia address both match.
pub fn extract(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for m in URL_IN_TEXT.find_iter(text) {
        let token = m.as_str().trim_end_matches(TRAILING_PUNCTUATION);
        let mut candidates = vec![token];
        if let Some(open) = token.strip_suffix(')') {
            candidates.push(open.trim_end_matches(TRAILING_PUNCTUATION));
        }
        for c in candidates {
            if let Some(n) = normalize(c)
                && !found.contains(&n)
            {
                found.push(n);
            }
        }
    }
    found
}

/// Why a tainted `web_fetch` of an unseen address is refused, for the model.
pub const REFUSAL: &str = "Not fetched: this address did not come from a search result, a page you fetched or the owner's own message, and this chat has read outside text, so HQ will not send a URL you composed. Search for it and fetch a result, or ask the owner to paste the link.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_form_drops_fragment_default_port_and_case_in_the_host() {
        assert_eq!(
            normalize("HTTPS://Docs.Example.com:443/a/B?q=1#top").as_deref(),
            Some("https://docs.example.com/a/B?q=1")
        );
    }

    #[test]
    fn userinfo_and_other_schemes_are_never_trusted() {
        assert_eq!(normalize("https://user:pw@x.example/"), None);
        assert_eq!(normalize("https://user@x.example/"), None);
        assert_eq!(normalize("ftp://x.example/"), None);
        assert_eq!(normalize("not a url"), None);
    }

    #[test]
    fn a_different_query_or_host_label_is_a_different_address() {
        let seen = normalize("https://x.example/").unwrap();
        assert_ne!(normalize("https://x.example/?d=notes").unwrap(), seen);
        assert_ne!(normalize("https://c2VjcmV0.x.example/").unwrap(), seen);
        assert_ne!(
            normalize("https://x.example./").unwrap(),
            seen,
            "trailing dot"
        );
    }

    #[test]
    fn urls_are_found_in_prose_and_markdown() {
        let text = "See https://a.example/p?x=1. Also [wiki](https://en.example/wiki/Foo_(bar)) and <https://b.example/z>, ok";
        let found = extract(text);
        for want in [
            "https://a.example/p?x=1",
            "https://en.example/wiki/Foo_(bar)",
            "https://b.example/z",
        ] {
            assert!(found.iter().any(|f| f == want), "{want} in {found:?}");
        }
    }
}
