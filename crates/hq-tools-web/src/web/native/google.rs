//! Google results without a key, through the Programmable Search "element"
//! endpoint and the public partner search id that SearxNG's `google cse`
//! engine also uses. Two requests: a script that hands out a short-lived
//! token, then a JSONP search that needs it. Responses are Google's own web
//! index, which is what makes this the strongest single source in the pool.

use super::super::*;
use super::engines::{BROWSER_USER_AGENT, Engine, accept_language, freshness_secs, result};
use super::images_code::image_snippet;

/// The public partner id (blackle.com) SearxNG ships for this engine.
const CX: &str = "partner-pub-8993703457585266:4862972284";
const TOKEN_TTL: Duration = Duration::from_secs(3600);
const PAGE_SIZE: usize = 20;
const CONSENT_COOKIE: &str = "CONSENT=YES+";
const REFERER: &str = "https://cse.google.com/";
pub(super) const MAX_PAGE: u32 = 5;
/// Google throttles a burst quickly ("unusual traffic" after a few dozen
/// searches from one address), so searches are spaced out. Tests skip the wait.
const MIN_INTERVAL: Duration = Duration::from_millis(if cfg!(test) { 0 } else { 1000 });

static LAST_REQUEST: std::sync::LazyLock<tokio::sync::Mutex<Option<Instant>>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));

/// Hold the next search until `MIN_INTERVAL` after the previous one. Callers
/// queue on the lock, so parallel searches are serialised rather than bursting.
pub(super) async fn pace() {
    let mut last = LAST_REQUEST.lock().await;
    if let Some(at) = *last {
        let wait = MIN_INTERVAL.saturating_sub(at.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
    *last = Some(Instant::now());
}

#[derive(Clone)]
struct Token {
    cse_tok: String,
    cselibv: String,
    exp: String,
    fetched: Instant,
}

/// `cse_tok`, `cselibv` and `exp` belong together, so they are cached as one.
/// Keyed by base URL, so a test server never shares a token with production.
static TOKENS: std::sync::LazyLock<Mutex<HashMap<String, Token>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn cached_token(base: &str) -> Option<Token> {
    let tokens = TOKENS.lock().ok()?;
    tokens
        .get(base)
        .filter(|t| t.fetched.elapsed() < TOKEN_TTL)
        .cloned()
}

pub(super) fn forget_token(base: &str) {
    if let Ok(mut tokens) = TOKENS.lock() {
        tokens.remove(base);
    }
}

async fn token(client: &Client, base: &str, lang: &str) -> Result<Token, ProviderError> {
    if let Some(token) = cached_token(base) {
        return Ok(token);
    }
    let request = client
        .get(format!("{base}/cse/cse.js"))
        .query(&[("cx", CX)])
        .header("User-Agent", BROWSER_USER_AGENT)
        .header("Accept", "*/*")
        .header("Accept-Language", lang)
        .header("Cookie", CONSENT_COOKIE);
    let body = get_body(request).await?;
    let token = parse_token(&body)?;
    if let Ok(mut tokens) = TOKENS.lock() {
        tokens.insert(base.to_string(), token.clone());
    }
    Ok(token)
}

/// The script ends with `({...});`; the options object is the last such call.
fn parse_token(script: &str) -> Result<Token, ProviderError> {
    let (start, end) = script
        .rfind("({")
        .zip(script.rfind("});"))
        .ok_or("no options in the search script")?;
    let object = script
        .get(start + 1..=end)
        .filter(|_| start < end)
        .ok_or("search script options are malformed")?;
    let opts: Value = serde_json::from_str(object)
        .map_err(|e| format!("search script options are not JSON: {e}"))?;
    let cse_tok = opts["cse_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or("failed to obtain cse token")?;
    let exp: Vec<&str> = opts["exp"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Ok(Token {
        cse_tok: cse_tok.to_string(),
        cselibv: opts["cselibVersion"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        exp: exp.join(","),
        fetched: Instant::now(),
    })
}

/// The search request. Argument order follows SearxNG's, which Google has
/// tolerated; `searchtype=image` turns the same endpoint into image search.
pub(super) async fn fetch(
    client: &Client,
    base: &str,
    query: &str,
    opts: &SearchOptions,
    images: bool,
) -> Result<Value, ProviderError> {
    let lang = accept_language(opts);
    let token = token(client, base, &lang).await?;
    let mut params: Vec<(&str, String)> = vec![
        ("rsz", "filtered_cse".into()),
        ("num", PAGE_SIZE.to_string()),
        ("hl", opts.language.clone().unwrap_or_else(|| "en".into())),
        ("cselibv", token.cselibv.clone()),
        ("cx", CX.into()),
        ("q", query.trim().to_string()),
        ("safe", "off".into()),
        ("cse_tok", token.cse_tok.clone()),
        ("callback", "_".into()),
        ("rurl", String::new()),
        ("searchtype", if images { "image" } else { "" }.into()),
    ];
    if let Some(f) = opts.freshness {
        let end = chrono::Utc::now();
        let start = end - chrono::Duration::seconds(freshness_secs(f));
        params.push((
            "sort",
            format!("date:r:{}:{}", start.format("%Y%m%d"), end.format("%Y%m%d")),
        ));
    }
    if let Some(country) = &opts.country {
        params.push(("gl", country.clone()));
    }
    if !token.exp.is_empty() {
        params.push(("exp", token.exp.clone()));
    }
    let start = opts.page.saturating_sub(1) as usize * PAGE_SIZE;
    if start > 0 {
        params.push(("start", start.to_string()));
    }
    let request = client
        .get(format!("{base}/cse/element/v1"))
        .query(&params)
        .header("User-Agent", BROWSER_USER_AGENT)
        .header("Accept", "*/*")
        .header("Accept-Language", lang)
        .header("Cookie", CONSENT_COOKIE)
        .header("Referer", REFERER);
    let body = match get_body(request).await {
        Ok(body) => body,
        Err(e) => {
            // A refusal may mean the token was rejected, so the next search starts over.
            if matches!(e.class, FailureClass::AccessDenied | FailureClass::Captcha) {
                forget_token(base);
            }
            return Err(e);
        }
    };
    let json = unwrap_jsonp(&body)?;
    if let Some(error) = json.get("error") {
        let message = error["message"].as_str().unwrap_or("unknown error");
        // A rejected token is dropped so the next search fetches a fresh one.
        forget_token(base);
        return Err(if error["code"].as_u64() == Some(429) {
            ProviderError::of(FailureClass::RateLimited, format!("google cse: {message}"))
        } else {
            ProviderError::new(format!("google cse: {message}"))
        });
    }
    Ok(json)
}

/// `_({...});` to the object inside.
fn unwrap_jsonp(body: &str) -> Result<Value, ProviderError> {
    let (start, end) = body
        .find('{')
        .zip(body.rfind('}'))
        .ok_or("response is not JSONP")?;
    let object = body
        .get(start..=end)
        .filter(|_| start < end)
        .ok_or("response is not JSONP")?;
    serde_json::from_str(object).map_err(|e| ProviderError::new(format!("malformed JSONP: {e}")))
}

pub(super) fn parse(engine: Engine, body: &Value) -> Result<Vec<SearchResult>, String> {
    // Google leaves `results` out when nothing matched, as SearxNG assumes.
    let items = body["results"].as_array().map_or(&[][..], Vec::as_slice);
    Ok(items
        .iter()
        .filter_map(|item| match engine {
            Engine::GoogleCseImages => image_item(item),
            _ => web_item(item),
        })
        .collect())
}

fn web_item(item: &Value) -> Option<SearchResult> {
    let url = web_url(item["unescapedUrl"].as_str()?)?;
    let title = item["titleNoFormatting"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let content = item["contentNoFormatting"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Some(result(title, url, content))
}

fn image_item(item: &Value) -> Option<SearchResult> {
    let image_url = web_url(item["unescapedUrl"].as_str()?)?;
    let page_url = web_url(item["originalContextUrl"].as_str()?)?;
    let title = item["titleNoFormatting"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let dims = item["width"]
        .as_str()
        .and_then(|w| w.parse().ok())
        .zip(item["height"].as_str().and_then(|h| h.parse().ok()));
    let about = item["contentNoFormatting"].as_str().unwrap_or_default();
    let format = item["fileFormat"]
        .as_str()
        .and_then(|f| f.rsplit('/').next())
        .unwrap_or_default();
    Some(result(
        title,
        page_url,
        image_snippet(&image_url, dims, format, "", about),
    ))
}

fn web_url(url: &str) -> Option<String> {
    (url.starts_with("https://") || url.starts_with("http://")).then(|| url.to_string())
}

#[cfg(test)]
pub(super) fn parse_token_for_test(script: &str) -> Result<(), ProviderError> {
    parse_token(script).map(|_| ())
}

#[cfg(test)]
pub(super) fn unwrap_jsonp_for_test(body: &str) -> Result<(), ProviderError> {
    unwrap_jsonp(body).map(|_| ())
}
