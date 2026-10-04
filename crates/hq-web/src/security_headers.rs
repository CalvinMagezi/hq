//! Response headers that keep a browser from running content the server did
//! not mean to be active: CSP for the PWA shell, nosniff, no referrer, no framing.

use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use std::path::Path;

/// The prerendered shell carries a few inline bootstrap scripts, so they are
/// allowed by hash instead of with `'unsafe-inline'`. The hashes are read from
/// `index.html` when the router is built; a new build needs a restart, which a
/// deploy already does. Styles keep `'unsafe-inline'` because React and the
/// syntax highlighter emit style attributes. `'wasm-unsafe-eval'` is for the
/// highlighter and pdf.js WebAssembly; `unsafe-eval` stays off.
pub(crate) fn content_security_policy(static_dir: &Path) -> HeaderValue {
    let index = static_dir.join("index.html");
    let shell = std::fs::read(&index).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_else(|e| {
        tracing::error!(path = %index.display(), error = %e, "web: cannot read the PWA shell; the CSP will block its inline scripts and the app will not start");
        String::new()
    });
    let found = inline_script_hashes(&shell);
    if found.is_empty() && !shell.is_empty() {
        tracing::error!(path = %index.display(), "web: the PWA shell has no inline scripts to allow; check the build");
    }
    let hashes: String = found.iter().map(|h| format!(" '{h}'")).collect();
    let policy = format!(
        "default-src 'self'; \
         script-src 'self' 'wasm-unsafe-eval'{hashes}; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; \
         font-src 'self' data:; \
         media-src 'self' blob:; \
         {CONNECT_SRC}; \
         worker-src 'self' blob:; \
         manifest-src 'self'; \
         object-src 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         frame-ancestors 'none'"
    );
    HeaderValue::from_str(&policy).expect("CSP is ASCII")
}

const CONNECT_SRC: &str = "connect-src 'self' blob:";

/// Index of the `>` that ends the tag starting at `tag[0]`, skipping quoted attribute values.
fn find_tag_end(tag: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in tag.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// Whether a start tag (without the closing `>`) has a `src` attribute, whatever its
/// position, spacing or case.
fn has_src_attribute(tag: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut name = String::new();
    let attrs = tag
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest);
    for c in attrs.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c.is_whitespace() || c == '=' || c == '/' => {
                if name.eq_ignore_ascii_case("src") {
                    return true;
                }
                name.clear();
            }
            None => name.push(c),
        }
    }
    name.eq_ignore_ascii_case("src")
}

/// The CSP for one response: the base policy plus the request's own host for the
/// chat socket, because older Safari does not treat `'self'` as covering `ws:`.
/// Only a plain host[:port] is trusted; anything else keeps the base policy.
pub(crate) fn policy_for_host(base: &HeaderValue, host: Option<&str>) -> HeaderValue {
    let plain = |h: &&str| {
        !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    };
    let Some(host) = host.filter(plain) else {
        return base.clone();
    };
    let Ok(text) = base.to_str() else {
        return base.clone();
    };
    let widened = text.replace(
        CONNECT_SRC,
        &format!("{CONNECT_SRC} ws://{host} wss://{host}"),
    );
    HeaderValue::from_str(&widened).unwrap_or_else(|_| base.clone())
}

/// A browser hashes script text as the HTML parser produced it, not the raw bytes:
/// CRLF and lone CR become LF, and NUL becomes U+FFFD. The caller reads the file
/// as lossy UTF-8, so invalid bytes are already U+FFFD.
pub(crate) fn normalize_script_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "\u{FFFD}")
}

/// `sha256-<base64>` source expressions for every inline `<script>` body.
pub(crate) fn inline_script_hashes(html: &str) -> Vec<String> {
    const OPEN: &str = "<script";
    let mut hashes = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find(OPEN) {
        let after = &rest[open..];
        let Some(tag_end) = find_tag_end(after) else {
            break;
        };
        if !after[OPEN.len()..].starts_with(|c: char| c.is_whitespace() || c == '>') {
            rest = &after[OPEN.len()..];
            continue;
        }
        let body_start = tag_end + 1;
        let Some(close) = after[body_start..].find("</script>") else {
            break;
        };
        if !has_src_attribute(&after[..tag_end]) {
            let body = normalize_script_text(&after[body_start..body_start + close]);
            let digest = Sha256::digest(body.as_bytes());
            hashes.push(format!("sha256-{}", STANDARD.encode(digest)));
        }
        rest = &after[body_start + close + "</script>".len()..];
    }
    hashes
}

const STATIC_HEADERS: [(HeaderName, &str); 3] = [
    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (header::REFERRER_POLICY, "no-referrer"),
    (header::X_FRAME_OPTIONS, "DENY"),
];

/// Sets the headers on every response unless a handler already chose its own
/// (the vault-asset route sets a stricter `Content-Security-Policy: sandbox`).
pub(crate) async fn set_security_headers(
    State(csp): State<HeaderValue>,
    req: Request,
    next: Next,
) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut res = next.run(req).await;
    let csp = policy_for_host(&csp, host.as_deref());
    let headers = res.headers_mut();
    for (name, value) in STATIC_HEADERS {
        headers
            .entry(name)
            .or_insert(HeaderValue::from_static(value));
    }
    headers
        .entry(header::CONTENT_SECURITY_POLICY)
        .or_insert(csp);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_cover_inline_scripts_only() {
        let html = r#"<script>var a=1</script><script src="/x.js"></script><script type="module" async="">import("/m.js")</script>"#;
        let hashes = inline_script_hashes(html);
        assert_eq!(hashes.len(), 2);
        let want = format!("sha256-{}", STANDARD.encode(Sha256::digest(b"var a=1")));
        assert_eq!(hashes[0], want);
    }

    /// Independent reference for the HTML preprocessing rules, char by char.
    fn reference_preprocess(raw: &str) -> String {
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    out.push('\n');
                }
                '\0' => out.push('\u{FFFD}'),
                c => out.push(c),
            }
        }
        out
    }

    fn sha(text: &str) -> String {
        format!(
            "sha256-{}",
            STANDARD.encode(Sha256::digest(text.as_bytes()))
        )
    }

    #[test]
    fn hashes_use_the_text_the_parser_produces() {
        let body = "a\r\nb\rc\0d\ne";
        let html = format!("<script>{body}</script>");
        let hashes = inline_script_hashes(&html);
        assert_eq!(hashes, [sha(&reference_preprocess(body))]);
        assert_eq!(reference_preprocess(body), "a\nb\nc\u{FFFD}d\ne");
        assert_ne!(
            hashes[0],
            sha(body),
            "raw-byte hash would never match in a browser"
        );
    }

    #[test]
    fn a_shell_with_nul_bytes_on_disk_is_hashed_after_normalisation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            b"<script>x\0y\xff\r\nz</script>",
        )
        .unwrap();
        let csp = content_security_policy(dir.path());
        let want = sha(&reference_preprocess("x\0y\u{FFFD}\r\nz"));
        assert!(csp.to_str().unwrap().contains(&format!("'{want}'")));
    }

    #[test]
    fn src_detection_ignores_position_spacing_and_case() {
        let html = "<script\n  type=\"module\"\n  SRC=\"/a.js\"></script><script async src='/b.js'></script><script data-x=\" src=\">inline()</script><scripted>x</scripted>";
        let hashes = inline_script_hashes(html);
        assert_eq!(hashes.len(), 1, "{hashes:?}");
        let want = format!("sha256-{}", STANDARD.encode(Sha256::digest(b"inline()")));
        assert_eq!(hashes[0], want);
    }

    #[test]
    fn socket_hosts_are_added_only_for_plain_hosts() {
        let base = HeaderValue::from_static("default-src 'self'; connect-src 'self' blob:; x");
        let ok = policy_for_host(&base, Some("hq.example.ts.net:8443"));
        assert!(
            ok.to_str()
                .unwrap()
                .contains("blob: ws://hq.example.ts.net:8443 wss://hq.example.ts.net:8443;")
        );
        for bad in ["a b", "x;script-src *", "", "a,b"] {
            assert_eq!(policy_for_host(&base, Some(bad)), base, "{bad}");
        }
        assert_eq!(policy_for_host(&base, None), base);
    }

    #[test]
    fn policy_is_strict_and_lists_the_shell_hashes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<script>boot()</script>").unwrap();
        let csp = content_security_policy(dir.path());
        let csp = csp.to_str().unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("object-src 'none'"));
        assert!(csp.contains("sha256-"));
        assert!(!csp.contains("'unsafe-eval'"));
        let script_src = csp
            .split(';')
            .find(|d| d.trim().starts_with("script-src"))
            .unwrap();
        assert!(!script_src.contains("'unsafe-inline'"));
    }
}
