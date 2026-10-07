use super::*;

pub(super) const JINA_READER_BASE: &str = "https://r.jina.ai/";
// A genuinely empty extraction (nav labels only, no real copy) vs. a real
// page that just happens to be short, like example.com's ~150-char body —
// verified against example.com live: 150ish chars extracted from 559 bytes
// of HTML, which must NOT trip this on length alone.
pub(super) const SPA_NEAR_EMPTY_TEXT_CHARS: usize = 40;
pub(super) const SPA_TEXT_TO_HTML_RATIO: f64 = 0.03;
pub(super) const SPA_MIN_HTML_BYTES_FOR_RATIO_CHECK: usize = 500;

pub(super) const METHOD_HTML: &str = "html-to-text";
pub(super) const METHOD_ARTICLE: &str = "article-extract";
pub(super) const METHOD_EMBEDDED: &str = "embedded-data";
pub(super) const METHOD_TEXT: &str = "plain-text";
pub(super) const METHOD_PDF_TEXT: &str = "pdf-text-layer";
pub(super) const METHOD_PDF_OCR: &str = "pdf-ocr";
pub(super) const METHOD_JINA: &str = "jina-reader";

/// A fetched page with its provenance. `content` may be truncated; the cache
/// keeps the full text.
#[derive(Debug, Clone, Serialize)]
pub struct FetchedPage {
    /// The requested URL after the https upgrade.
    pub url: String,
    /// Where the content actually came from after redirects.
    pub final_url: String,
    pub content_type: String,
    /// `article-extract`, `html-to-text`, `embedded-data`, `plain-text`, `pdf-text-layer`,
    /// `pdf-ocr` or `jina-reader`.
    pub method: &'static str,
    pub content: String,
    pub total_chars: usize,
    pub truncated: bool,
    /// Partial or degraded extraction, e.g. OCR limits or a third-party render.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl FetchedPage {
    pub(super) fn truncate_to(mut self, max_chars: usize) -> Self {
        if self.content.len() > max_chars {
            let boundary = self.content.floor_char_boundary(max_chars);
            self.content.truncate(boundary);
            self.truncated = true;
        }
        self
    }

    /// Text for agent consumption: a provenance line, the content, and a truncation marker.
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "[Fetched {} | {} | extracted via {}]\n",
            self.final_url,
            if self.content_type.is_empty() {
                "unknown type"
            } else {
                &self.content_type
            },
            self.method
        );
        for note in &self.notes {
            out.push_str(&format!("[Note: {note}]\n"));
        }
        out.push('\n');
        out.push_str(&self.content);
        if self.truncated {
            out.push_str(&format!(
                "\n\n[Content truncated at {} chars ({} total)]",
                self.content.len(),
                self.total_chars
            ));
        }
        out
    }
}

/// Heuristic for "this HTML page's extracted text looks like an empty
/// client-rendered shell rather than real content" — e.g. a Next.js/React
/// SPA that only fills its DOM after JS runs, which a plain GET + HTML-to-text
/// pass never executes. Two independent signals: near-zero text regardless of
/// page size, or (only once the page is big enough for the ratio to be
/// meaningful) too little text relative to a large raw HTML payload — the
/// signature of a big JS bundle with almost no server-rendered copy. A short
/// but real page (small HTML, modest text, healthy ratio) must trip neither.
pub(super) fn looks_like_empty_spa_shell(extracted_text: &str, raw_html_len: usize) -> bool {
    let trimmed_len = extracted_text.trim().chars().count();
    if trimmed_len < SPA_NEAR_EMPTY_TEXT_CHARS {
        return true;
    }
    if raw_html_len > SPA_MIN_HTML_BYTES_FOR_RATIO_CHECK {
        let ratio = extracted_text.trim().len() as f64 / raw_html_len as f64;
        if ratio < SPA_TEXT_TO_HTML_RATIO {
            return true;
        }
    }
    false
}

/// Parsing hostile markup (tens of thousands of nested elements) is slow, so
/// extraction runs off the async workers and is abandoned past this.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(3);

async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    tokio::time::timeout(EXTRACT_TIMEOUT, tokio::task::spawn_blocking(work))
        .await
        .ok()?
        .ok()?
}

/// Environment switch for the third-party Jina Reader fallback: `0` turns it off.
pub(super) const JINA_ENV: &str = "HQ_WEB_FETCH_JINA";

/// A privacy opt-out, so any plausible "off" spelling disables it.
pub(super) fn jina_base_from_env() -> &'static str {
    let off = std::env::var(JINA_ENV).is_ok_and(|v| {
        matches!(v.trim().to_lowercase().as_str(), "" | "0" | "false" | "off" | "no")
    });
    if off { "" } else { JINA_READER_BASE }
}

/// Render a URL through Jina Reader, which executes JS server-side and
/// returns markdown. No API key; only the URL is sent.
pub(super) async fn fetch_via_jina(jina_base: &str, url: &str) -> Result<String> {
    let resp = get_client()
        .get(format!("{jina_base}{url}"))
        .timeout(JINA_TIMEOUT)
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("Jina Reader returned HTTP {}", resp.status());
    }
    let bytes = read_limited(resp, MAX_BODY_BYTES).await?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BodyKind {
    Html,
    Pdf,
    Text,
    Binary,
}

/// Media types rejected from headers alone, before downloading the body.
pub(super) fn is_binary_media_type(content_type: &str) -> bool {
    ["image/", "audio/", "video/", "application/zip", "font/"]
        .iter()
        .any(|t| content_type.contains(t))
}

/// PDFs are recognised by magic bytes as well as by type, since many servers
/// label them `application/octet-stream`.
pub(super) fn classify_body(content_type: &str, body: &[u8]) -> BodyKind {
    if body.starts_with(b"%PDF-") || content_type.contains("application/pdf") {
        return BodyKind::Pdf;
    }
    if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        return BodyKind::Html;
    }
    const SNIFF_BYTES: usize = 1024;
    let looks_binary = body.iter().take(SNIFF_BYTES).any(|b| *b == 0);
    if is_binary_media_type(content_type)
        || content_type.contains("application/octet-stream")
        || looks_binary
    {
        return BodyKind::Binary;
    }
    BodyKind::Text
}

/// Read a body without ever buffering more than `limit` bytes.
pub(super) async fn read_limited(mut resp: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if let Some(len) = resp.content_length()
        && len as usize > limit
    {
        bail!("Response body too large: {len} bytes (limit: {limit} bytes)");
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() + chunk.len() > limit {
            bail!("Response body too large: over {limit} bytes");
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Extract text from PDF bytes via `hq-convert`: the text layer first, OCR
/// when that comes back empty (a scanned PDF).
pub(super) async fn extract_pdf(bytes: &[u8]) -> Result<(String, &'static str)> {
    let path = std::env::temp_dir().join(format!("hq-web-fetch-{}.pdf", uuid::Uuid::new_v4()));
    tokio::fs::write(&path, bytes).await?;
    let result = extract_pdf_file(&path).await;
    let _ = tokio::fs::remove_file(&path).await;
    result
}

pub(super) async fn extract_pdf_file(path: &Path) -> Result<(String, &'static str)> {
    use hq_convert::{InboundConverter, OcrEngine};

    let text_layer = match InboundConverter::new() {
        Ok(converter) => tokio::time::timeout(PDF_TEXT_TIMEOUT, converter.convert(path))
            .await
            .ok()
            .and_then(Result::ok),
        Err(_) => None,
    };
    if let Some(text) = text_layer.filter(|t| t.trim().chars().count() >= MIN_PDF_TEXT_CHARS) {
        return Ok((text, METHOD_PDF_TEXT));
    }
    match tokio::time::timeout(PDF_OCR_TIMEOUT, OcrEngine::extract_pdf(path)).await {
        Err(_) => bail!(
            "PDF has no text layer and OCR timed out after {}s",
            PDF_OCR_TIMEOUT.as_secs()
        ),
        Ok(Ok(text)) if !text.trim().is_empty() => Ok((text, METHOD_PDF_OCR)),
        Ok(Ok(_)) => bail!("PDF has no extractable text, even after OCR"),
        Ok(Err(e)) => bail!("PDF has no text layer and OCR is unavailable on this host: {e}"),
    }
}

/// Fetch a URL and return its text with provenance.
/// Caches results for 15 minutes. Auto-upgrades HTTP to HTTPS.
pub async fn web_fetch(url: &str, max_chars: usize) -> Result<FetchedPage> {
    let url = upgrade_url(url);
    validate_url(&url)?;

    if let Some(cached) = cache_get(&url) {
        debug!(url = %url, "web_fetch cache hit");
        return Ok(cached.truncate_to(max_chars));
    }

    let fetcher = Fetcher {
        client: &FETCH_CLIENT,
        timeout: FETCH_TIMEOUT,
        jina_base: jina_base_from_env(),
    };
    let page = fetcher.fetch(&url).await?;
    cache_put(&url, &page);
    Ok(page.truncate_to(max_chars))
}

/// Where an uncached fetch goes. Tests point it at local servers.
pub(super) struct Fetcher<'a> {
    pub(super) client: &'a Client,
    /// Must match the timeout `client` was built with; only used in error text.
    pub(super) timeout: Duration,
    /// Empty disables the Jina fallback.
    pub(super) jina_base: &'a str,
}

/// reqwest's `Display` for a redirect error drops the policy's reason
/// ("redirect blocked: ..."), which only survives in the source chain.
pub(super) fn error_chain(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

impl Fetcher<'_> {
    pub(super) async fn fetch(&self, url: &str) -> Result<FetchedPage> {
        debug!(url = %url, "fetching web page");
        let response = self.client.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                anyhow::anyhow!(
                    "Timed out after {}s fetching {url}",
                    self.timeout.as_secs_f32()
                )
            } else {
                anyhow::anyhow!("Fetch failed for {url}: {}", error_chain(&e))
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            bail!("HTTP {} for {}", status, url);
        }
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if is_binary_media_type(&content_type) {
            bail!(
                "Cannot extract text from binary content type: {}. Use bash + curl for binary downloads.",
                content_type
            );
        }

        let bytes = read_limited(response, MAX_BODY_BYTES).await?;
        let mut notes = Vec::new();
        let (text, method) = self
            .extract(url, &final_url, &content_type, &bytes, &mut notes)
            .await?;
        if text.trim().is_empty() {
            notes.push("extraction produced no text".into());
        }
        Ok(FetchedPage {
            url: url.to_string(),
            final_url,
            content_type,
            method,
            total_chars: text.len(),
            content: text,
            truncated: false,
            notes,
        })
    }

    async fn extract(
        &self,
        url: &str,
        final_url: &str,
        content_type: &str,
        bytes: &[u8],
        notes: &mut Vec<String>,
    ) -> Result<(String, &'static str)> {
        match classify_body(content_type, bytes) {
            BodyKind::Binary => bail!(
                "Cannot extract text from binary content ({}). Use bash + curl for binary downloads.",
                if content_type.is_empty() {
                    "no content type"
                } else {
                    content_type
                }
            ),
            BodyKind::Pdf => {
                let (text, method) = extract_pdf(bytes).await?;
                if method == METHOD_PDF_OCR {
                    notes.push("scanned PDF read with OCR; only the first pages are covered and text may contain recognition errors".into());
                }
                Ok((text, method))
            }
            BodyKind::Text => Ok((String::from_utf8_lossy(bytes).to_string(), METHOD_TEXT)),
            BodyKind::Html => Ok(self.extract_html(url, final_url, bytes, notes).await),
        }
    }

    /// Main-content extraction first. A page that comes back as an empty
    /// client-rendered shell is recovered from its embedded JSON-LD or
    /// framework data, then through Jina Reader if enabled; the original
    /// extraction is kept if neither has anything.
    async fn extract_html(
        &self,
        url: &str,
        final_url: &str,
        bytes: &[u8],
        notes: &mut Vec<String>,
    ) -> (String, &'static str) {
        let html: std::sync::Arc<str> = String::from_utf8_lossy(bytes).into_owned().into();
        let text = html_to_text(&html, HTML_TEXT_WIDTH);
        if !looks_like_empty_spa_shell(&text, bytes.len()) {
            let (page, base, plain_len) = (html.clone(), final_url.to_string(), text.len());
            let article = off_thread(move || extract_article(&page, &base, plain_len)).await;
            return match article {
                Some(article) => (article.render(), METHOD_ARTICLE),
                None => (text, METHOD_HTML),
            };
        }
        let page = html.clone();
        if let Some(embedded) = off_thread(move || embedded_text(&page)).await {
            notes.push("page is client-rendered; text recovered from the data embedded in its HTML, so it may be a summary rather than the full page".into());
            return (embedded, METHOD_EMBEDDED);
        }
        if self.jina_base.is_empty() {
            notes.push("page looks client-rendered and the Jina Reader fallback is disabled; text may be incomplete".into());
            return (text, METHOD_HTML);
        }
        debug!(url = %url, "web_fetch looks like an empty SPA shell, trying Jina Reader");
        match fetch_via_jina(self.jina_base, final_url).await {
            Ok(jina) if !jina.trim().is_empty() => {
                notes.push("page rendered by the third-party Jina Reader (r.jina.ai), which received only the URL".into());
                (jina, METHOD_JINA)
            }
            Ok(_) | Err(_) => {
                notes.push("page looks client-rendered and Jina Reader could not render it; text may be incomplete".into());
                (text, METHOD_HTML)
            }
        }
    }
}

/// Auto-upgrade http:// to https:// for safety.
pub(super) fn upgrade_url(url: &str) -> String {
    if let Some(stripped) = url.strip_prefix("http://") {
        format!("https://{}", stripped)
    } else {
        url.to_string()
    }
}

/// Convert HTML to readable plain text.
pub fn html_to_text(html: &str, width: usize) -> String {
    html2text::from_read(html.as_bytes(), width).unwrap_or_else(|_| html.to_string())
}
