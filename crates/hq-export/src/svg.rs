//! A small allowlist sanitiser for inline SVG found in a note's raw HTML.
//!
//! Raw HTML is otherwise dropped from every export. A chart drawn as inline
//! SVG is the one exception: [`extract_svgs`] rebuilds each `<svg>` element
//! from scratch, keeping only known shapes and presentation attributes, so
//! nothing the note author wrote is passed through verbatim. Scripts,
//! `foreignObject`, event handlers, external references and `url(...)` in any
//! value are gone. The wrapping HTML (a `div`, say) is not kept: the exporter
//! puts the chart in its own container.

/// Elements that survive. Everything else loses its tag (children are still
/// visited), and the dangerous ones lose their content too.
const ELEMENTS: &[&str] = &[
    "svg", "g", "rect", "line", "path", "circle", "ellipse", "polyline", "polygon", "text",
    "tspan", "title", "desc",
];

/// Elements whose whole subtree is discarded.
const DROP_WITH_CONTENT: &[&str] = &[
    "script",
    "foreignobject",
    "style",
    "iframe",
    "object",
    "embed",
    "animate",
    "set",
    "animatetransform",
    "animatemotion",
    "defs",
    "symbol",
    "use",
    "image",
    "filter",
    "pattern",
    "mask",
    "clippath",
    "marker",
    "lineargradient",
    "radialgradient",
];

/// Attributes kept, by lowercase name, with the canonical spelling to emit.
const ATTRS: &[(&str, &str)] = &[
    ("fill", "fill"),
    ("stroke", "stroke"),
    ("stroke-width", "stroke-width"),
    ("stroke-linecap", "stroke-linecap"),
    ("stroke-linejoin", "stroke-linejoin"),
    ("stroke-dasharray", "stroke-dasharray"),
    ("opacity", "opacity"),
    ("fill-opacity", "fill-opacity"),
    ("stroke-opacity", "stroke-opacity"),
    ("x", "x"),
    ("y", "y"),
    ("width", "width"),
    ("height", "height"),
    ("viewbox", "viewBox"),
    ("preserveaspectratio", "preserveAspectRatio"),
    ("font-size", "font-size"),
    ("font-family", "font-family"),
    ("font-weight", "font-weight"),
    ("role", "role"),
    ("aria-label", "aria-label"),
    ("aria-hidden", "aria-hidden"),
    ("rx", "rx"),
    ("ry", "ry"),
    ("d", "d"),
    ("points", "points"),
    ("cx", "cx"),
    ("cy", "cy"),
    ("r", "r"),
    ("x1", "x1"),
    ("y1", "y1"),
    ("x2", "x2"),
    ("y2", "y2"),
    ("dx", "dx"),
    ("dy", "dy"),
    ("text-anchor", "text-anchor"),
    ("dominant-baseline", "dominant-baseline"),
    ("transform", "transform"),
    ("style", "style"),
    ("href", "href"),
    ("xlink:href", "xlink:href"),
];

/// CSS properties allowed inside a `style` attribute.
const STYLE_PROPS: &[&str] = &[
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "opacity",
    "fill-opacity",
    "stroke-opacity",
    "font-size",
    "font-family",
    "font-weight",
    "text-anchor",
    "dominant-baseline",
];

/// Larger than this and the chart is skipped rather than embedded.
const MAX_SVG_BYTES: usize = 256 * 1024;

/// Every `<svg>` element in `html`, sanitised, in document order.
pub fn extract_svgs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_ci(html, "<svg", pos) {
        // `<svgfoo` is not an svg tag.
        let after = html[start + 4..].chars().next();
        if !matches!(after, Some(c) if c.is_ascii_whitespace() || c == '>' || c == '/') {
            pos = start + 4;
            continue;
        }
        let (clean, end) = sanitise_from(html, start);
        if let Some(c) = clean.filter(|c| c.len() <= MAX_SVG_BYTES) {
            out.push(c);
        }
        pos = end.max(start + 4);
    }
    out
}

fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if h.len() < n.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    closing: bool,
    self_closing: bool,
}

/// Parse the tag starting at `s[i] == '<'`. Returns the tag and the index
/// after its closing `>`, or `None` when it is not a well-formed tag.
fn parse_tag(s: &str, i: usize) -> Option<(Tag, usize)> {
    let b = s.as_bytes();
    let mut p = i + 1;
    let closing = b.get(p) == Some(&b'/');
    if closing {
        p += 1;
    }
    let name_start = p;
    while p < b.len() && (b[p].is_ascii_alphanumeric() || matches!(b[p], b'-' | b':' | b'_')) {
        p += 1;
    }
    if p == name_start {
        return None;
    }
    let name = s[name_start..p].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while p < b.len() && b[p].is_ascii_whitespace() {
            p += 1;
        }
        match b.get(p)? {
            b'>' => {
                p += 1;
                break;
            }
            b'/' => {
                self_closing = true;
                p += 1;
            }
            _ => {
                let a0 = p;
                while p < b.len()
                    && !b[p].is_ascii_whitespace()
                    && !matches!(b[p], b'=' | b'>' | b'/')
                {
                    p += 1;
                }
                if p == a0 {
                    p += 1;
                    continue;
                }
                let aname = s[a0..p].to_ascii_lowercase();
                while p < b.len() && b[p].is_ascii_whitespace() {
                    p += 1;
                }
                let mut value = String::new();
                if b.get(p) == Some(&b'=') {
                    p += 1;
                    while p < b.len() && b[p].is_ascii_whitespace() {
                        p += 1;
                    }
                    match b.get(p)? {
                        q @ (b'"' | b'\'') => {
                            let q = *q;
                            let v0 = p + 1;
                            let rel = b[v0..].iter().position(|&c| c == q)?;
                            value = s[v0..v0 + rel].to_owned();
                            p = v0 + rel + 1;
                        }
                        _ => {
                            let v0 = p;
                            while p < b.len() && !b[p].is_ascii_whitespace() && b[p] != b'>' {
                                p += 1;
                            }
                            value = s[v0..p].to_owned();
                        }
                    }
                }
                attrs.push((aname, value));
                self_closing = false;
            }
        }
    }
    Some((
        Tag {
            name,
            attrs,
            closing,
            self_closing,
        },
        p,
    ))
}

/// Sanitise the svg element opening at `start`. Returns the clean markup (if
/// the element closed properly) and the index to resume scanning from.
fn sanitise_from(s: &str, start: usize) -> (Option<String>, usize) {
    let mut out = String::new();
    let mut open: Vec<&'static str> = Vec::new();
    let mut i = start;
    let mut skip: Option<(String, usize)> = None; // (name, depth)
    while i < s.len() {
        let Some(lt) = s[i..].find('<').map(|n| i + n) else {
            break;
        };
        if skip.is_none() && !open.is_empty() {
            out.push_str(&escape_text(&s[i..lt]));
        }
        if s[lt..].starts_with("<!--") {
            match s[lt..].find("-->") {
                Some(n) => i = lt + n + 3,
                None => return (None, s.len()),
            }
            continue;
        }
        let Some((tag, next)) = parse_tag(s, lt) else {
            // A stray `<`: drop it and carry on after it.
            i = lt + 1;
            continue;
        };
        i = next;
        if let Some((name, depth)) = skip.as_mut() {
            if tag.name == *name {
                if tag.closing {
                    if *depth == 0 {
                        skip = None;
                    } else {
                        *depth -= 1;
                    }
                } else if !tag.self_closing {
                    *depth += 1;
                }
            }
            continue;
        }
        if open.is_empty() && !(tag.name == "svg" && !tag.closing) {
            continue;
        }
        if !tag.closing && DROP_WITH_CONTENT.contains(&tag.name.as_str()) {
            if !tag.self_closing {
                skip = Some((tag.name, 0));
            }
            continue;
        }
        let Some(&canon) = ELEMENTS.iter().find(|e| **e == tag.name) else {
            continue;
        };
        if tag.closing {
            if let Some(idx) = open.iter().rposition(|e| *e == canon) {
                while open.len() > idx {
                    let e = open.pop().unwrap_or(canon);
                    out.push_str(&format!("</{e}>"));
                }
                if open.is_empty() {
                    return (Some(out), i);
                }
            }
            continue;
        }
        out.push('<');
        out.push_str(canon);
        if canon == "svg" && open.is_empty() {
            out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
        }
        for (name, value) in &tag.attrs {
            if let Some(clean) = clean_attr(name, value) {
                out.push_str(&format!(" {}=\"{}\"", clean.0, escape_attr(&clean.1)));
            }
        }
        if tag.self_closing {
            out.push_str("/>");
        } else {
            out.push('>');
            open.push(canon);
        }
    }
    (None, s.len())
}

fn clean_attr(name: &str, value: &str) -> Option<(&'static str, String)> {
    let (_, canon) = ATTRS.iter().find(|(n, _)| *n == name)?;
    let v = value.trim();
    if canon.ends_with("href") {
        // Only a reference inside the same document.
        return (v.starts_with('#') && safe_value(v)).then(|| (*canon, v.to_owned()));
    }
    if *canon == "style" {
        let style = clean_style(v);
        return (!style.is_empty()).then_some((*canon, style));
    }
    safe_value(v).then(|| (*canon, v.to_owned()))
}

/// A value with nothing that can reach outside the document or run code.
fn safe_value(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    !(l.contains("url(")
        || l.contains("javascript:")
        || l.contains("data:")
        || l.contains("expression")
        || l.contains("@import")
        || l.contains('\\')
        || l.contains('<')
        || l.contains("/*"))
}

fn clean_style(v: &str) -> String {
    v.split(';')
        .filter_map(|decl| {
            let (prop, val) = decl.split_once(':')?;
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim();
            (STYLE_PROPS.contains(&prop.as_str()) && !val.is_empty() && safe_value(val))
                .then(|| format!("{prop}:{val}"))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Escape markup characters, leaving a well-formed entity (`&amp;`, `&#35;`)
/// alone: the source is HTML, so its text is already escaped.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        match c {
            '&' if is_entity(&s[i..]) => out.push('&'),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn is_entity(s: &str) -> bool {
    let body = s[1..].split(';').next().filter(|_| s[1..].contains(';'));
    matches!(body, Some(b) if (1..=10).contains(&b.len())
        && b.trim_start_matches('#').chars().all(|c| c.is_ascii_alphanumeric())
        && !b.trim_start_matches('#').is_empty())
}

/// Every `&` is escaped, entities included: a browser decodes an entity in an attribute before it reads
/// the value as CSS, so `ur&#108;(` would otherwise turn back into `url(` after `safe_value` passed it.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = r##"<div style="margin:1em"><svg viewBox="0 0 200 100" role="img" aria-label="Prices" width="200" height="100"><title>Bars</title><g fill="#336" transform="translate(4,4)"><rect x="0" y="10" width="40" height="80" rx="3" style="fill:#c00;stroke:#000"/><line x1="0" y1="95" x2="200" y2="95" stroke="#999"/><circle cx="10" cy="10" r="4"/><path d="M0 0L10 10"/><text x="5" y="9" font-size="9" font-family="sans-serif" text-anchor="middle">UGX 5 &amp; up</text></g></svg></div>"##;

    #[test]
    fn chart_is_kept() {
        let s = extract_svgs(CHART);
        assert_eq!(s.len(), 1);
        let c = &s[0];
        for want in [
            "<svg xmlns=\"http://www.w3.org/2000/svg\"",
            "viewBox=\"0 0 200 100\"",
            "<rect x=\"0\"",
            "style=\"fill:#c00;stroke:#000\"",
            "<circle",
            "<path d=\"M0 0L10 10\"/>",
            "text-anchor=\"middle\"",
            "UGX 5 &amp; up",
            "<title>Bars</title>",
            "</svg>",
        ] {
            assert!(c.contains(want), "missing {want}: {c}");
        }
        assert!(!c.contains("<div"), "wrapper is not kept: {c}");
    }

    #[test]
    fn dangerous_content_is_removed() {
        let evil = r##"<svg width="10" onload="alert(1)"><script>alert(1)</script><foreignObject><div onclick="x()">hi</div></foreignObject><a href="https://evil.example/"><rect x="1" onclick="x()" fill="url(https://evil.example/x)" style="fill:url(#a);stroke:red;background:url(http://e)"/></a><rect href="javascript:alert(1)" xlink:href="https://e.example/x" stroke="red"/><rect href="#ok"/><use href="https://e.example/s.svg#a"/><image href="https://e.example/i.png"/><style>@import 'x';</style></svg>"##;
        let s = extract_svgs(evil);
        assert_eq!(s.len(), 1);
        let c = s[0].to_ascii_lowercase();
        for bad in [
            "script",
            "foreignobject",
            "onload",
            "onclick",
            "alert",
            "evil.example",
            "url(",
            "e.example",
            "javascript",
            "<use",
            "<image",
            "<style",
            "@import",
            "<a ",
        ] {
            assert!(!c.contains(bad), "{bad} survived: {c}");
        }
        assert!(c.contains("href=\"#ok\""), "{c}");
        assert!(c.contains("stroke:red"), "{c}");
    }

    #[test]
    fn an_entity_cannot_rebuild_a_blocked_value() {
        let sneaky = r##"<svg><rect style="fill:ur&#108;(https://e.example/x)" fill="ur&#x6c;(https://e.example/y)" stroke="red"/><text aria-label="A &amp; B">x</text></svg>"##;
        let c = extract_svgs(sneaky).remove(0);
        assert!(
            !c.contains("&#"),
            "an entity survived inside an attribute: {c}"
        );
        assert!(c.contains("aria-label=\"A &amp;amp; B\""), "{c}");
    }

    #[test]
    fn unclosed_or_oversized_svg_is_skipped() {
        assert!(extract_svgs("<svg><rect/>").is_empty());
        assert!(extract_svgs("<svgx></svgx> no chart here").is_empty());
        let big = format!("<svg><text>{}</text></svg>", "a".repeat(MAX_SVG_BYTES + 1));
        assert!(extract_svgs(&big).is_empty());
    }
}
