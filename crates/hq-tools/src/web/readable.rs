//! Main-content extraction for `web_fetch`: the article body without the
//! navigation, footers and sidebars `html2text` keeps, plus the text a
//! client-rendered page ships inside its own JSON when the DOM is empty.

use scraper::{ElementRef, Html, Node, Selector};
use serde_json::Value;

/// Below this many characters an extraction is not trusted as the page's content.
const MIN_ARTICLE_CHARS: usize = 200;
/// If the article is under 1/N of what a plain conversion found, content was probably cut.
const MIN_ARTICLE_SHARE_DENOMINATOR: usize = 10;
const MIN_PROSE_STRING_CHARS: usize = 80;
const MAX_EMBEDDED_CHARS: usize = 20_000;
const NEXT_DATA_ID: &str = "__NEXT_DATA__";
/// Longer targets are tracking or prefilled-form URLs that cost tokens and mean nothing.
const MAX_LINK_URL_CHARS: usize = 200;

const NOISE_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "nav", "footer", "iframe", "button",
    "select", "dialog",
];
/// Page chrome outside an `<article>`, but real content (intro, callout, form of a quiz) inside one.
const NOISE_UNLESS_IN_ARTICLE: &[&str] = &["header", "aside", "form"];
/// Deeper nesting than any real page; stops hostile markup from overflowing the stack.
const MAX_DEPTH: usize = 256;
/// Class or id fragments that mark chrome rather than content.
const NOISE_HINTS: &[&str] = &[
    "sidebar",
    "cookie",
    "newsletter",
    "advert",
    "popup",
    "modal",
    "breadcrumb",
    "social-share",
];
const PARAGRAPH_TAGS: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "pre",
    "blockquote",
    "table",
    "ul",
    "ol",
    "figure",
];
const LINE_TAGS: &[&str] = &[
    "div", "section", "article", "main", "li", "tr", "br", "hr", "dd", "dt",
];

pub(super) struct Article {
    pub(super) title: Option<String>,
    pub(super) byline: Option<String>,
    pub(super) published: Option<String>,
    pub(super) text: String,
}

impl Article {
    /// The text an agent reads: a heading and byline line when known, then the body.
    pub(super) fn render(&self) -> String {
        let meta: Vec<&str> = [self.byline.as_deref(), self.published.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let meta_line = (!meta.is_empty()).then(|| meta.join(" | "));
        let (heading, rest) = match self.text.split_once('\n') {
            Some((first, rest)) if first.starts_with('#') => {
                (Some(first.to_string()), rest.trim_start())
            }
            _ => (None, self.text.as_str()),
        };
        let heading = heading.or_else(|| self.title.as_ref().map(|t| format!("# {t}")));
        let mut parts: Vec<&str> = Vec::new();
        if let Some(h) = &heading {
            parts.push(h);
        }
        if let Some(m) = &meta_line {
            parts.push(m);
        }
        parts.push(rest);
        parts.join("\n\n")
    }
}

fn sel(css: &str) -> Selector {
    Selector::parse(css).expect("static CSS selector")
}

/// Pull the main content out of an HTML page. `None` means no confident
/// container was found, and the caller should use a plain conversion.
/// `plain_len` is the length of that plain conversion, used to catch an
/// extraction that threw most of the page away.
pub(super) fn extract_article(html: &str, base: &str, plain_len: usize) -> Option<Article> {
    let doc = Html::parse_document(html);
    let base = reqwest::Url::parse(base).ok();
    let text = best_container(&doc, base.as_ref())?;
    if text.chars().count() < MIN_ARTICLE_CHARS
        || text.len() * MIN_ARTICLE_SHARE_DENOMINATOR < plain_len
    {
        return None;
    }
    Some(Article {
        title: meta_content(&doc, &["meta[property=\"og:title\"]"])
            .or_else(|| first_text(&doc, "title")),
        byline: meta_content(
            &doc,
            &["meta[name=\"author\"]", "meta[property=\"article:author\"]"],
        )
        .or_else(|| first_text(&doc, "[rel=\"author\"]")),
        published: meta_content(
            &doc,
            &[
                "meta[property=\"article:published_time\"]",
                "meta[name=\"date\"]",
                "meta[itemprop=\"datePublished\"]",
            ],
        )
        .or_else(|| {
            doc.select(&sel("time[datetime]"))
                .next()
                .and_then(|t| t.value().attr("datetime"))
                .map(String::from)
        }),
        text,
    })
}

/// Semantic containers first (`article`, `main`), then the block with the most
/// paragraph text, then the whole body. Returns its cleaned text.
fn best_container(doc: &Html, base: Option<&reqwest::Url>) -> Option<String> {
    let semantic_selector = sel("article, main, [role=\"main\"]");
    let mut best: Option<String> = None;
    for el in doc.select(&semantic_selector) {
        let text = render(el, base);
        if best.as_ref().is_none_or(|b| text.len() > b.len()) {
            best = Some(text);
        }
    }
    if best
        .as_ref()
        .is_some_and(|t| t.chars().count() >= MIN_ARTICLE_CHARS)
    {
        return best;
    }
    let block_selector = sel("div, section");
    let paragraph_rich = doc
        .select(&block_selector)
        .filter(|el| !is_noise(el))
        .map(|el| (direct_paragraph_chars(&el), el))
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, _)| *score)
        .map(|(_, el)| el);
    if let Some(el) = paragraph_rich {
        let text = render(el, base);
        if text.chars().count() >= MIN_ARTICLE_CHARS {
            return Some(text);
        }
    }
    let body = doc.select(&sel("body")).next()?;
    Some(render(body, base))
}

fn direct_paragraph_chars(el: &ElementRef<'_>) -> usize {
    el.children()
        .filter_map(ElementRef::wrap)
        .filter(|c| c.value().name() == "p")
        .map(|p| p.text().map(|t| t.trim().len()).sum::<usize>())
        .sum()
}

fn is_noise(el: &ElementRef<'_>) -> bool {
    let v = el.value();
    if NOISE_TAGS.contains(&v.name()) {
        return true;
    }
    if NOISE_UNLESS_IN_ARTICLE.contains(&v.name()) && !in_article(el) {
        return true;
    }
    // Whole class tokens, so `sidebar` and `sidebar-left` match but a layout
    // wrapper such as `has-sidebar` does not.
    let hinted = |s: &str| {
        s.split_whitespace().any(|token| {
            let token = token.to_lowercase();
            NOISE_HINTS.iter().any(|h| {
                token == *h
                    || token.starts_with(&format!("{h}-"))
                    || token.starts_with(&format!("{h}_"))
            })
        })
    };
    v.attr("class").is_some_and(hinted)
        || v.attr("id").is_some_and(hinted)
        || v.attr("aria-hidden") == Some("true")
        || v.attr("hidden").is_some()
}

fn in_article(el: &ElementRef<'_>) -> bool {
    el.ancestors()
        .filter_map(ElementRef::wrap)
        .any(|a| a.value().name() == "article")
}

fn render(el: ElementRef<'_>, base: Option<&reqwest::Url>) -> String {
    let mut out = String::new();
    walk(el, base, 0, &mut out);
    clean_text(&out)
}

fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run == 1 && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn ensure_break(out: &mut String, blank_line: bool) {
    if out.is_empty() {
        return;
    }
    let want = if blank_line { "\n\n" } else { "\n" };
    while !out.ends_with(want) {
        out.push('\n');
    }
}

fn walk(el: ElementRef<'_>, base: Option<&reqwest::Url>, depth: usize, out: &mut String) {
    if depth > MAX_DEPTH {
        return;
    }
    for child in el.children() {
        match child.value() {
            Node::Text(t) => push_words(out, t),
            Node::Element(_) => {
                let Some(child_el) = ElementRef::wrap(child) else {
                    continue;
                };
                if is_noise(&child_el) {
                    continue;
                }
                walk_element(child_el, base, depth, out);
            }
            _ => {}
        }
    }
}

fn walk_element(el: ElementRef<'_>, base: Option<&reqwest::Url>, depth: usize, out: &mut String) {
    let name = el.value().name();
    if matches!(name, "td" | "th") && !out.is_empty() && !out.ends_with('\n') {
        out.push_str(" | ");
    }
    if name == "pre" {
        ensure_break(out, true);
        out.push_str(el.text().collect::<String>().trim_matches('\n'));
        ensure_break(out, true);
        return;
    }
    if name == "a" {
        push_link(el, base, out);
        return;
    }
    let paragraph = PARAGRAPH_TAGS.contains(&name);
    let line = LINE_TAGS.contains(&name);
    if paragraph || line {
        ensure_break(out, paragraph);
    }
    match name {
        "h1" => out.push_str("# "),
        "h2" => out.push_str("## "),
        "h3" | "h4" | "h5" | "h6" => out.push_str("### "),
        "li" => out.push_str("- "),
        _ => {}
    }
    walk(el, base, depth + 1, out);
    if paragraph || line {
        ensure_break(out, paragraph);
    }
}

fn push_words(out: &mut String, text: &str) {
    let mut words = text.split_whitespace().peekable();
    if words.peek().is_none() {
        if text.chars().any(char::is_whitespace) && !out.is_empty() && !out.ends_with(['\n', ' ']) {
            out.push(' ');
        }
        return;
    }
    if text.starts_with(char::is_whitespace) && !out.is_empty() && !out.ends_with(['\n', ' ']) {
        out.push(' ');
    }
    out.push_str(&words.collect::<Vec<_>>().join(" "));
    if text.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

/// Anchor text with its absolute target, so an agent can follow links it reads.
fn push_link(el: ElementRef<'_>, base: Option<&reqwest::Url>, out: &mut String) {
    let label: String = el
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        return;
    }
    let target = el
        .value()
        .attr("href")
        .and_then(|h| base.and_then(|b| b.join(h).ok()))
        .filter(|u| {
            matches!(u.scheme(), "http" | "https") && u.as_str().len() <= MAX_LINK_URL_CHARS
        });
    if !out.is_empty() && !out.ends_with(['\n', ' ', '(']) {
        out.push(' ');
    }
    match target {
        Some(url) => out.push_str(&format!("[{label}]({url})")),
        None => out.push_str(&label),
    }
    out.push(' ');
}

fn meta_content(doc: &Html, selectors: &[&str]) -> Option<String> {
    selectors.iter().find_map(|s| {
        doc.select(&sel(s))
            .filter_map(|m| m.value().attr("content"))
            .map(|c| c.trim().to_string())
            .find(|c| !c.is_empty())
    })
}

fn first_text(doc: &Html, css: &str) -> Option<String> {
    doc.select(&sel(css))
        .map(|e| {
            e.text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .find(|t| !t.is_empty())
}

/// Text a client-rendered page ships inside the HTML: schema.org JSON-LD
/// (`articleBody`, `description`) and framework data (`__NEXT_DATA__`) prose.
/// `None` unless there is enough of it to be the page's content.
pub(super) fn embedded_text(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let mut parts: Vec<String> = Vec::new();
    for script in doc.select(&sel("script[type=\"application/ld+json\"]")) {
        if let Ok(json) = serde_json::from_str::<Value>(&script.text().collect::<String>()) {
            collect_json_ld(&json, &mut parts);
        }
    }
    if let Some(script) = doc.select(&sel(&format!("script#{NEXT_DATA_ID}"))).next()
        && let Ok(json) = serde_json::from_str::<Value>(&script.text().collect::<String>())
    {
        collect_prose(&json, &mut parts);
    }
    if let Some(desc) = meta_content(
        &doc,
        &[
            "meta[property=\"og:description\"]",
            "meta[name=\"description\"]",
        ],
    ) {
        parts.push(desc);
    }
    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| seen.insert(p.clone()));
    let mut text = parts.join("\n\n");
    if text.chars().count() < MIN_ARTICLE_CHARS {
        return None;
    }
    if text.len() > MAX_EMBEDDED_CHARS {
        text.truncate(text.floor_char_boundary(MAX_EMBEDDED_CHARS));
    }
    Some(text)
}

fn collect_json_ld(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for key in ["headline", "description", "articleBody", "text"] {
                if let Some(s) = map
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    out.push(s.to_string());
                }
            }
            map.values().for_each(|c| collect_json_ld(c, out));
        }
        Value::Array(items) => items.iter().for_each(|c| collect_json_ld(c, out)),
        _ => {}
    }
}

fn collect_prose(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            let t = s.trim();
            let prose = t.chars().count() >= MIN_PROSE_STRING_CHARS
                && t.contains(' ')
                && !t.starts_with("http")
                && !t.starts_with('{')
                && !t.contains("function(");
            if prose {
                out.push(t.to_string());
            }
        }
        Value::Object(map) => map.values().for_each(|c| collect_prose(c, out)),
        Value::Array(items) => items.iter().for_each(|c| collect_prose(c, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTICLE_PAGE: &str = r#"<html><head><title>Fallback title</title>
<meta property="og:title" content="Why Rust Wins">
<meta name="author" content="A. Writer">
<meta property="article:published_time" content="2026-09-01T10:00:00Z">
</head><body>
<header><a href="/">Home</a> <a href="/about">About</a></header>
<nav><ul><li><a href="/a">Nav one</a></li><li><a href="/b">Nav two</a></li></ul></nav>
<div class="sidebar"><p>Subscribe to our weekly newsletter for a lot of marketing copy that should be dropped.</p></div>
<article>
<h1>Why Rust Wins</h1>
<p>Rust gives memory safety without a garbage collector, which matters for services that must keep latency predictable under load for many hours at a time.</p>
<p>The borrow checker is strict at first, but it turns a class of production incidents into compile errors. See the <a href="/book">official book</a> for details.</p>
<pre>fn main() {
    println!("hi");
}</pre>
<ul><li>Fast</li><li>Safe</li></ul>
</article>
<footer>Copyright footer text and legal links that should be dropped.</footer>
<script>window.tracking = "should never appear";</script>
</body></html>"#;

    fn extract(html: &str) -> Option<Article> {
        extract_article(html, "https://example.com/post", 0)
    }

    #[test]
    fn keeps_the_article_and_drops_page_chrome() {
        let a = extract(ARTICLE_PAGE).expect("article found");
        let out = a.render();
        assert!(out.starts_with("# Why Rust Wins"), "{out}");
        assert!(out.contains("A. Writer | 2026-09-01T10:00:00Z"), "{out}");
        assert!(
            out.contains("memory safety without a garbage collector"),
            "{out}"
        );
        assert!(
            out.contains("[official book](https://example.com/book)"),
            "{out}"
        );
        assert!(
            out.contains("fn main() {\n    println!(\"hi\");\n}"),
            "{out}"
        );
        assert!(out.contains("- Fast\n- Safe"), "{out}");
        for dropped in [
            "Nav one",
            "newsletter",
            "Copyright footer",
            "should never appear",
            "Home",
        ] {
            assert!(!out.contains(dropped), "{dropped} leaked into:\n{out}");
        }
    }

    #[test]
    fn title_is_not_repeated_when_the_body_already_starts_with_it() {
        let out = extract(ARTICLE_PAGE).unwrap().render();
        assert_eq!(out.matches("Why Rust Wins").count(), 1, "{out}");
    }

    #[test]
    fn a_page_without_semantic_tags_uses_the_paragraph_rich_block() {
        let html = format!(
            "<html><body><div class=\"top\"><a href=\"/x\">menu</a></div><div id=\"content\">{}</div></body></html>",
            "<p>This is a long enough paragraph of real prose to count as the main content of the page and pass the threshold.</p>".repeat(3)
        );
        let out = extract(&html).expect("block found").render();
        assert!(out.contains("real prose"), "{out}");
        assert!(!out.contains("menu"), "{out}");
    }

    #[test]
    fn a_tiny_page_is_left_to_the_plain_converter() {
        assert!(extract("<html><body><p>Short.</p></body></html>").is_none());
    }

    #[test]
    fn an_extraction_that_threw_away_most_of_the_page_is_rejected() {
        assert!(extract_article(ARTICLE_PAGE, "https://example.com/", 1_000_000).is_none());
    }

    #[test]
    fn json_ld_article_body_is_recovered_from_an_empty_shell() {
        let body = "A long article body that only exists inside the structured data of this client rendered page, written out at length so it clears the minimum size. It keeps going with a second sentence about the topic, and a third one that adds even more detail for the reader to use.";
        let html = format!(
            r#"<html><head><script type="application/ld+json">{{"@type":"NewsArticle","headline":"Big news","articleBody":"{body}"}}</script></head><body><div id="root"></div></body></html>"#
        );
        let text = embedded_text(&html).expect("embedded text");
        assert!(text.contains("Big news") && text.contains(body), "{text}");
    }

    #[test]
    fn next_data_prose_is_recovered_and_urls_and_short_strings_are_not() {
        let prose = "This paragraph lives in the Next.js page props and is long enough to be treated as prose by the extractor for sure.";
        let html = format!(
            r#"<html><body><script id="__NEXT_DATA__" type="application/json">{{"props":{{"pageProps":{{"post":{{"body":"{prose}","slug":"a-b","image":"https://cdn.example.com/a.png"}}}}}}}}</script></body></html>"#
        );
        // One paragraph alone is under the minimum, so add the description meta.
        let html = html.replace("<body>", "<head><meta name=\"description\" content=\"A site description that is also fairly long so that the two together exceed the minimum extraction size for trust.\"></head><body>");
        let text = embedded_text(&html).expect("embedded text");
        assert!(text.contains(prose), "{text}");
        assert!(
            !text.contains("cdn.example.com") && !text.contains("a-b"),
            "{text}"
        );
    }

    #[test]
    fn link_text_from_separate_elements_is_not_glued_and_huge_urls_are_dropped() {
        let long = "x".repeat(400);
        let html = format!(
            r#"<html><body><main><p>{filler}</p><a href="/a"><h3>Headline</h3><p>Summary text</p></a><a href="/r?q={long}">Report a problem</a></main></body></html>"#,
            filler = "Enough ordinary prose to pass the minimum size for trusting the extraction result. ".repeat(4)
        );
        let out = extract(&html).expect("article").render();
        assert!(
            out.contains("[Headline Summary text](https://example.com/a)"),
            "{out}"
        );
        assert!(
            out.contains("Report a problem") && !out.contains(&long),
            "{out}"
        );
    }

    #[test]
    fn layout_classes_that_merely_mention_a_hint_do_not_hide_content() {
        let html = format!(
            r#"<html><body><main><div class="content-with-sidebar has-sidebar"><p>{}</p></div><div class="sidebar"><p>Promo that should go.</p></div></main></body></html>"#,
            "The real body text sits inside a layout wrapper whose class only mentions a sidebar. "
                .repeat(4)
        );
        let out = extract(&html).expect("article").render();
        assert!(out.contains("real body text"), "{out}");
        assert!(!out.contains("Promo that should go"), "{out}");
    }

    #[test]
    fn header_and_aside_inside_an_article_are_kept_but_page_level_ones_are_not() {
        let html = format!(
            r#"<html><body><header>Site banner</header><article><header><h1>Inside Title</h1></header><aside>A callout worth reading.</aside><p>{}</p></article></body></html>"#,
            "Paragraph text that makes the article long enough to be trusted by the extraction. "
                .repeat(4)
        );
        let out = extract(&html).expect("article").render();
        assert!(
            out.contains("Inside Title") && out.contains("A callout worth reading."),
            "{out}"
        );
        assert!(!out.contains("Site banner"), "{out}");
    }

    #[test]
    fn minified_table_cells_are_separated() {
        let html = format!(
            r#"<html><body><main><p>{}</p><table><tr><th>Name</th><th>Age</th></tr><tr><td>Ann</td><td>31</td></tr></table></main></body></html>"#,
            "Enough surrounding prose so that the extraction clears the minimum size to be trusted. ".repeat(4)
        );
        let out = extract(&html).expect("article").render();
        assert!(
            out.contains("Name | Age") && out.contains("Ann | 31"),
            "{out}"
        );
    }

    #[test]
    fn non_latin_pages_are_measured_in_characters_not_bytes() {
        let html = format!(
            "<html><body><main><p>{}</p></main></body></html>",
            "これは日本語の文章で、バイト数ではなく文字数で長さを数える必要があります。".repeat(10)
        );
        assert!(extract(&html).is_some());
    }

    #[test]
    fn pathological_nesting_does_not_overflow_the_stack() {
        let depth = 3_000;
        let html = format!(
            "<html><body><main>{}<p>{}</p>{}</main></body></html>",
            "<div>".repeat(depth),
            "text ".repeat(100),
            "</div>".repeat(depth)
        );
        // Reaching the end without a stack overflow is the assertion.
        let _ = extract_article(&html, "https://example.com/", 0);
    }

    #[test]
    fn too_little_embedded_text_is_not_trusted() {
        assert!(
            embedded_text(
                r#"<html><head><meta name="description" content="Short."></head></html>"#
            )
            .is_none()
        );
    }
}
