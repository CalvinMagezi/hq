use regex::Regex;

pub(super) fn sel(css: &str) -> scraper::Selector {
    scraper::Selector::parse(css).expect("static CSS selector")
}

pub(super) fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strip tags and decode entities from a fragment such as a search snippet.
pub(super) fn html_text(fragment: &str) -> String {
    let doc = scraper::Html::parse_fragment(fragment);
    collapse_ws(&doc.root_element().text().collect::<Vec<_>>().join(""))
}

// The feeds read here (Bing News RSS, the arXiv Atom API) are flat, so tag
// matching is enough. ponytail: no nested same-name tags; use a real XML
// reader if a feed with them is ever added.
pub(super) fn blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let re =
        Regex::new(&format!(r"(?s)<{tag}(?:\s[^>]*)?>(.*?)</{tag}>")).expect("static tag regex");
    re.captures_iter(xml)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect()
}

/// Inner text of the first `<tag>`, with a CDATA wrapper removed.
pub(super) fn tag_text(block: &str, tag: &str) -> Option<String> {
    let inner = blocks(block, tag).into_iter().next()?.trim();
    let inner = inner
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
        .unwrap_or(inner);
    Some(inner.trim().to_string())
}

/// Value of `attr` on the first `<tag ...>` whose attribute text contains `must_contain`.
pub(super) fn attr_value(block: &str, tag: &str, must_contain: &str, attr: &str) -> Option<String> {
    let tag_re = Regex::new(&format!(r"(?s)<{tag}\s([^>]*?)/?>")).expect("static tag regex");
    let attr_re = Regex::new(&format!(r#"{attr}="([^"]*)""#)).expect("static attr regex");
    tag_re
        .captures_iter(block)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .find(|attrs| attrs.contains(must_contain))
        .and_then(|attrs| attr_re.captures(attrs))
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}
