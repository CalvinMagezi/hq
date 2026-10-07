//! Mojeek, an independent index. It answers an unknown client with an Altcha
//! proof-of-work page: the server hands out a PBKDF2 puzzle, any client that
//! computes the answer gets a cookie that lasts weeks. Solving what a server
//! offers every visitor is ordinary client work; nothing here pretends to be a
//! person or hides what the client is.

use super::super::*;
use super::engines::{BROWSER_USER_AGENT, accept_language, element_text, result};
use super::markup::sel;
use super::super::state;
use ring::pbkdf2;
use std::num::NonZeroU32;

const MAX_COST: u32 = 100_000;
const MAX_KEY_LENGTH: usize = 64;
/// The puzzle space is small (hundreds of tries at cost 8000); this only bounds a hostile server.
const MAX_COUNTER: u32 = 200_000;
const SOLVE_TIMEOUT: Duration = Duration::from_secs(6);
const PAGE_SIZE: usize = 10;
const CHALLENGE_PATH: &str = "/captcha/challenge";
/// The challenge page loads the Altcha widget; results pages never do.
const CAPTCHA_MARKER: &str = "altcha";
const COOKIE_NAME: &str = "chllg";

/// The verification cookie per base URL, so a test server never shares one with production.
static COOKIES: std::sync::LazyLock<Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// The cookie lasts about a month; renew a little before that.
const COOKIE_TTL: Duration = Duration::from_secs(25 * 24 * 3600);

fn cookie_row(base: &str) -> String {
    format!("mojeek-cookie:{base}")
}

fn cached_cookie(base: &str) -> Option<String> {
    if let Some(cookie) = COOKIES.lock().ok()?.get(base).cloned() {
        return Some(cookie);
    }
    let stored = state::is_persistable(base)
        .then(|| state::production()?.get(&cookie_row(base)))
        .flatten()?;
    store_in_memory(base, stored.clone());
    Some(stored)
}

fn store_in_memory(base: &str, cookie: String) {
    if let Ok(mut c) = COOKIES.lock() {
        c.insert(base.to_string(), cookie);
    }
}

fn store_cookie(base: &str, cookie: String) {
    if let (true, Some(store)) = (state::is_persistable(base), state::production()) {
        store.put(&cookie_row(base), &cookie, COOKIE_TTL);
    }
    store_in_memory(base, cookie);
}

pub(super) fn forget_cookie(base: &str) {
    if let Ok(mut c) = COOKIES.lock() {
        c.remove(base);
    }
    if let (true, Some(store)) = (state::is_persistable(base), state::production()) {
        store.remove(&cookie_row(base));
    }
}

pub(super) async fn fetch(
    client: &Client,
    base: &str,
    query: &str,
    opts: &SearchOptions,
) -> Result<Value, ProviderError> {
    let mut body = search(client, base, query, opts).await?;
    if is_challenge(&body) {
        forget_cookie(base);
        // Detached, so a pool that stops waiting for this engine still leaves the cookie for the next search.
        let (client_for_verify, base_for_verify, lang) =
            (client.clone(), base.to_string(), accept_language(opts));
        tokio::spawn(async move { verify(&client_for_verify, &base_for_verify, &lang).await })
            .await
            .map_err(|e| ProviderError::new(format!("verification task failed: {e}")))??;
        body = search(client, base, query, opts).await?;
        if is_challenge(&body) {
            return Err(ProviderError::of(
                FailureClass::Captcha,
                "still challenged after solving the proof of work",
            ));
        }
    }
    Ok(Value::String(body))
}

fn is_challenge(body: &str) -> bool {
    body.contains(CAPTCHA_MARKER)
}

async fn search(
    client: &Client,
    base: &str,
    query: &str,
    opts: &SearchOptions,
) -> Result<String, ProviderError> {
    let mut params = vec![("q", query.trim().to_string())];
    let start = opts.page.saturating_sub(1) as usize * PAGE_SIZE;
    if start > 0 {
        params.push(("s", (start + 1).to_string()));
    }
    let mut request = client
        .get(format!("{base}/search"))
        .query(&params)
        .header("User-Agent", BROWSER_USER_AGENT)
        .header("Accept-Language", accept_language(opts));
    if let Some(cookie) = cached_cookie(base) {
        request = request.header("Cookie", format!("{COOKIE_NAME}={cookie}"));
    }
    get_body(request).await
}

/// The puzzle as Mojeek serves it from `/captcha/challenge`.
#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct Puzzle {
    parameters: Parameters,
    signature: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct Parameters {
    algorithm: String,
    cost: u32,
    #[serde(rename = "keyLength")]
    key_length: usize,
    #[serde(rename = "keyPrefix")]
    key_prefix: String,
    nonce: String,
    salt: String,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

async fn verify(client: &Client, base: &str, lang: &str) -> Result<(), ProviderError> {
    let challenge = client
        .get(format!("{base}{CHALLENGE_PATH}"))
        .header("User-Agent", BROWSER_USER_AGENT)
        .header("Accept-Language", lang);
    let puzzle: Puzzle = serde_json::from_str(&get_body(challenge).await?)
        .map_err(|e| ProviderError::new(format!("unreadable challenge: {e}")))?;
    let started = Instant::now();
    let solving = puzzle.clone();
    let give_up_at = started + SOLVE_TIMEOUT;
    let (counter, derived) =
        tokio::task::spawn_blocking(move || solve(&solving.parameters, give_up_at))
            .await
            .map_err(|e| ProviderError::new(format!("proof of work failed: {e}")))??;
    let payload = serde_json::json!({
        "challenge": {"parameters": puzzle.parameters, "signature": puzzle.signature},
        "solution": {
            "counter": counter,
            "derivedKey": derived,
            "time": started.elapsed().as_millis() as u64,
        },
    });
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
    let form = reqwest::multipart::Form::new().text("altcha", encoded);
    let resp = client
        .post(format!("{base}/captcha/verify"))
        .header("User-Agent", BROWSER_USER_AGENT)
        .header("X-Requested-With", "XMLHttpRequest")
        .multipart(form)
        .send()
        .await
        .map_err(|e| ProviderError::new(format!("verification failed: {}", e.without_url())))?;
    let cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(cookie_value)
        .ok_or_else(|| ProviderError::of(FailureClass::Captcha, "verification gave no cookie"))?;
    store_cookie(base, cookie);
    Ok(())
}

fn cookie_value(set_cookie: &str) -> Option<String> {
    let pair = set_cookie.split(';').next()?;
    pair.strip_prefix(&format!("{COOKIE_NAME}="))
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_graphic() && b != b';'))
        .map(str::to_string)
}

/// Find the counter whose PBKDF2 key starts with the server's prefix. The
/// password is the nonce followed by the counter as four big-endian bytes.
fn solve(p: &Parameters, give_up_at: Instant) -> Result<(u32, String), ProviderError> {
    if p.algorithm != "PBKDF2/SHA-256" {
        return Err(ProviderError::new(format!(
            "unsupported challenge algorithm {}",
            p.algorithm
        )));
    }
    if p.cost == 0 || p.cost > MAX_COST || p.key_length == 0 || p.key_length > MAX_KEY_LENGTH {
        return Err(ProviderError::new("challenge parameters out of range"));
    }
    let nonce = hex::decode(&p.nonce).map_err(|_| "challenge nonce is not hex")?;
    let salt = hex::decode(&p.salt).map_err(|_| "challenge salt is not hex")?;
    let prefix = p.key_prefix.to_lowercase();
    let cost = NonZeroU32::new(p.cost).ok_or("challenge cost is zero")?;
    let mut key = vec![0u8; p.key_length];
    let mut password = nonce.clone();
    password.extend_from_slice(&[0; 4]);
    let tail = nonce.len();
    for counter in 0..MAX_COUNTER {
        // A blocking thread cannot be cancelled, so it stops itself.
        if Instant::now() >= give_up_at {
            return Err(ProviderError::new("proof of work took too long"));
        }
        password[tail..].copy_from_slice(&counter.to_be_bytes());
        pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, cost, &salt, &password, &mut key);
        let hex_key = hex::encode(&key);
        if hex_key.starts_with(&prefix) {
            return Ok((counter, hex_key));
        }
    }
    Err(ProviderError::new("no solution within the counter limit"))
}

pub(super) fn parse(html: &str) -> Vec<SearchResult> {
    let doc = scraper::Html::parse_document(html);
    let (block, title, snippet) = (sel("ul.results-standard > li"), sel("a.title"), sel("p.s"));
    doc.select(&block)
        .filter_map(|li| {
            let a = li.select(&title).next()?;
            let url = a.value().attr("href").filter(|h| h.starts_with("http"))?;
            let name = element_text(a);
            let snip = li
                .select(&snippet)
                .next()
                .map(element_text)
                .unwrap_or_default();
            Some(result(name, url.to_string(), snip))
        })
        .collect()
}

#[cfg(test)]
pub(super) const TEST_COUNTER: u32 = 7;

/// A puzzle whose answer is `TEST_COUNTER`, built the way the server does.
#[cfg(test)]
pub(super) fn test_puzzle(cost: u32) -> serde_json::Value {
    let (nonce, salt) = ([1u8, 2, 3, 4], [9u8, 8, 7]);
    let mut password = nonce.to_vec();
    password.extend_from_slice(&TEST_COUNTER.to_be_bytes());
    let mut key = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(cost).unwrap(),
        &salt,
        &password,
        &mut key,
    );
    serde_json::json!({
        "parameters": {
            "algorithm": "PBKDF2/SHA-256", "cost": cost, "keyLength": 32,
            "keyPrefix": hex::encode(&key[..8]), "nonce": hex::encode(nonce),
            "salt": hex::encode(salt), "expiresAt": 1,
        },
        "signature": "sig",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters(puzzle: &Value) -> Parameters {
        serde_json::from_value(puzzle["parameters"].clone()).unwrap()
    }

    #[test]
    fn solves_the_puzzle_the_server_built() {
        let (counter, key) = solve(
            &parameters(&test_puzzle(50)),
            Instant::now() + SOLVE_TIMEOUT,
        )
        .unwrap();
        assert_eq!(counter, TEST_COUNTER);
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn refuses_work_a_hostile_server_could_use_to_burn_cpu() {
        let mut p = parameters(&test_puzzle(50));
        p.cost = MAX_COST + 1;
        let later = Instant::now() + SOLVE_TIMEOUT;
        assert!(solve(&p, later).is_err());
        p.cost = 50;
        p.algorithm = "SCRYPT".into();
        assert!(solve(&p, later).is_err());
        p.algorithm = "PBKDF2/SHA-256".into();
        assert!(
            solve(&p, Instant::now()).is_err(),
            "an expired deadline stops the search"
        );
    }

    #[test]
    fn parses_results_from_a_real_page() {
        let html = include_str!("fixtures/mojeek-results.html");
        let results = parse(html);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert!(results[0].title.contains("Tokio"));
        assert!(results[0].snippet.contains("runtime"));
    }

    #[test]
    fn reads_the_verification_cookie() {
        assert_eq!(
            cookie_value("chllg=abc123; Path=/; Max-Age=100"),
            Some("abc123".into())
        );
        assert_eq!(cookie_value("other=1"), None);
    }
}
