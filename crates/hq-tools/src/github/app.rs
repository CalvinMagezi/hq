//! The person's own GitHub App: the manifest flow that creates it, the key that stays on this
//! device, and the short-lived tokens signed with it. No App or key belongs to the HQ project
//! (`docs/plans/mobile-helpers.md`).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use reqwest::Method;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{API_HOST_URL, USER_AGENT, records_root};

pub const KEY_FILE: &str = "app.pem";
pub const RECORD_FILE: &str = "app.json";
const GITHUB_WEB: &str = "https://github.com";
const NEW_APP_URL: &str = "https://github.com/settings/apps/new";
const HOMEPAGE: &str = env!("CARGO_PKG_REPOSITORY");
const ACCEPT_JSON: &str = "application/vnd.github+json";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const STATE_BYTES: usize = 32;
const NAME_SUFFIX_BYTES: usize = 3;
/// Long enough to read GitHub's page and press Create; GitHub's own code lasts an hour.
const FLOW_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PENDING_FLOWS: usize = 8;
/// A phone's clock can run behind GitHub's, so the token claims to start a minute ago.
const JWT_BACKDATE_SECS: u64 = 60;
/// GitHub refuses more than ten minutes ahead; two spare minutes cover a clock running fast.
const JWT_LIFETIME_SECS: u64 = 8 * 60;
const MAX_CODE_CHARS: usize = 200;
const PRIVATE_FILE_MODE: u32 = 0o600;
const PRIVATE_DIR_MODE: u32 = 0o700;
const PKCS1_LABEL: &str = "RSA PRIVATE KEY";
const PKCS8_LABEL: &str = "PRIVATE KEY";

/// What the App may ever do; GitHub enforces it, and Workflows is left out on purpose.
fn app_permissions() -> Value {
    json!({
        "contents": "write",
        "pull_requests": "write",
        "actions": "read",
        "checks": "read",
        "statuses": "read",
        "metadata": "read",
    })
}

/// The definition GitHub shows on its "Create GitHub App" page. `base` is this HQ's loopback
/// address. HQ cannot know the person's login before GitHub signs them in, so the name carries a
/// random suffix instead, which also keeps it unique across GitHub.
pub fn manifest(base: &str) -> Value {
    json!({
        "name": format!("HQ Coder {}", random_hex(NAME_SUFFIX_BYTES)),
        "url": HOMEPAGE,
        "redirect_url": format!("{base}{RETURN_PATH}"),
        "setup_url": format!("{base}{INSTALLED_PATH}"),
        "public": false,
        "default_permissions": app_permissions(),
        "default_events": [],
    })
}

pub const START_PATH: &str = "/github/connect/start";
pub const RETURN_PATH: &str = "/github/connect/return";
pub const INSTALLED_PATH: &str = "/github/connect/installed";

fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system random source failed");
    bytes
}

fn random_hex(len: usize) -> String {
    random_bytes(len)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Connection attempts HQ started. A `state` not in here is refused, and each one works once.
#[derive(Default)]
pub struct Flows(Mutex<HashMap<String, Instant>>);

impl Flows {
    /// Only owner paths may call this; the value it returns is the flow's whole capability.
    pub fn begin(&self) -> String {
        let state = URL_SAFE_NO_PAD.encode(random_bytes(STATE_BYTES));
        let mut flows = self.0.lock().unwrap_or_else(|e| e.into_inner());
        flows.retain(|_, born| born.elapsed() < FLOW_TTL);
        while flows.len() >= MAX_PENDING_FLOWS {
            let Some(oldest) = flows
                .iter()
                .min_by_key(|(_, b)| **b)
                .map(|(s, _)| s.clone())
            else {
                break;
            };
            flows.remove(&oldest);
        }
        flows.insert(state.clone(), Instant::now());
        state
    }

    /// Whether `state` is pending, without using it up: the start page may be reloaded.
    pub fn is_pending(&self, state: &str) -> bool {
        let flows = self.0.lock().unwrap_or_else(|e| e.into_inner());
        flows.get(state).is_some_and(|b| b.elapsed() < FLOW_TTL)
    }

    /// Uses up `state`. True only the first time, and only before it expires.
    pub fn take(&self, state: &str) -> bool {
        let mut flows = self.0.lock().unwrap_or_else(|e| e.into_inner());
        flows.remove(state).is_some_and(|b| b.elapsed() < FLOW_TTL)
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A browser page and the headers that keep it to itself.
pub struct Page {
    pub html: String,
    pub csp: String,
}

/// Headers every page of the flow is served with, beside its own `csp`.
pub const PAGE_HEADERS: [(&str, &str); 3] = [
    ("cache-control", "no-store"),
    ("referrer-policy", "no-referrer"),
    ("x-content-type-options", "nosniff"),
];

/// Sends the browser on to GitHub with the manifest. The button is there for a browser that
/// will not run the script.
pub fn start_page(state: &str, base: &str) -> Page {
    let nonce = random_hex(STATE_BYTES);
    let manifest = escape_html(&manifest(base).to_string());
    let action = format!("{NEW_APP_URL}?state={}", escape_html(state));
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>\
         <title>Connect GitHub</title>\
         <form id=f method=post action=\"{action}\"><input type=hidden name=manifest value=\"{manifest}\">\
         <button>Continue to GitHub</button></form>\
         <script nonce=\"{nonce}\">document.getElementById('f').submit()</script>"
    );
    let csp =
        format!("default-src 'none'; script-src 'nonce-{nonce}'; form-action https://github.com");
    Page { html, csp }
}

/// A plain message page for the end of the flow.
pub fn message_page(title: &str, body: &str) -> Page {
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>\
         <title>{0}</title><h1>{0}</h1><p>{1}</p>",
        escape_html(title),
        escape_html(body),
    );
    Page {
        html,
        csp: "default-src 'none'".to_string(),
    }
}

/// The step a token is for; each gets only the permissions of that step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Fetch,
    Publish,
    ReadCi,
}

impl Step {
    fn permissions(self) -> Value {
        match self {
            Step::Fetch => json!({"contents": "read"}),
            Step::Publish => json!({"contents": "write", "pull_requests": "write"}),
            Step::ReadCi => json!({"actions": "read", "checks": "read", "statuses": "read"}),
        }
    }
}

/// An installation token. Held in memory only, and never printable by accident.
pub struct AppToken(String);

impl AppToken {
    pub fn secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for AppToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppToken(<hidden>)")
    }
}

/// Everything HQ keeps about the App beside its key. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRecord {
    pub app_id: u64,
    pub client_id: String,
    pub slug: String,
    pub owner_login: String,
    pub owner_id: u64,
    pub owner_name: Option<String>,
    #[serde(default)]
    pub installation_id: Option<u64>,
    #[serde(default)]
    pub repository_selection: Option<String>,
    /// GitHub no longer accepts the key: the App or its key was deleted there.
    #[serde(default)]
    pub needs_reconnect: bool,
}

impl AppRecord {
    pub fn install_url(&self) -> String {
        format!("{GITHUB_WEB}/apps/{}/installations/new", self.slug)
    }

    pub fn settings_url(&self) -> String {
        format!("{GITHUB_WEB}/settings/apps/{}", self.slug)
    }

    /// The person as a commit author, so merged work counts on their profile.
    pub fn author(&self) -> Value {
        let name = self.owner_name.as_deref().unwrap_or(&self.owner_login);
        let email = format!(
            "{}+{}@users.noreply.github.com",
            self.owner_id, self.owner_login
        );
        json!({"name": name, "email": email})
    }
}

/// Where a connection stands, in the words the phone, the web app and the CLI all use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    NotConnected,
    NeedsInstall,
    Connected,
    Reconnect,
}

pub fn standing(record: Option<&AppRecord>) -> Standing {
    match record {
        None => Standing::NotConnected,
        Some(r) if r.needs_reconnect => Standing::Reconnect,
        Some(r) if r.installation_id.is_none() => Standing::NeedsInstall,
        Some(_) => Standing::Connected,
    }
}

/// The key and record under `<hq dir>/github/`: outside the vault, so `hq backup` never carries
/// them, and inside the folder `governance::secrets` denies to every file tool.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn standard() -> Self {
        Self::at(records_root())
    }

    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn load(&self) -> Option<AppRecord> {
        let bytes = std::fs::read(self.root.join(RECORD_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn key(&self) -> Result<String> {
        std::fs::read_to_string(self.root.join(KEY_FILE)).context("the GitHub App key is missing")
    }

    fn write_private(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let tmp = self
            .root
            .join(format!("{name}.{}.tmp", random_hex(STATE_BYTES)));
        let mut file = create_private(&self.root, &tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, self.root.join(name))?;
        Ok(())
    }

    pub fn save_record(&self, record: &AppRecord) -> Result<()> {
        self.write_private(RECORD_FILE, &serde_json::to_vec_pretty(record)?)
    }

    fn save_key(&self, pem: &str) -> Result<()> {
        self.write_private(KEY_FILE, pem.as_bytes())
    }

    /// Removes the key first: a record without a key is only a stale label.
    pub fn delete(&self) -> Result<()> {
        for name in [KEY_FILE, RECORD_FILE] {
            match std::fs::remove_file(self.root.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn create_private(dir: &Path, path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))?;
    Ok(std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)?)
}

/// Without owner-only file modes the key has no protection, so it is never written.
#[cfg(not(unix))]
fn create_private(_dir: &Path, _path: &Path) -> Result<std::fs::File> {
    bail!("connecting GitHub needs Linux, macOS or Android; on Windows run HQ under WSL2")
}

fn key_pair(pem: &str) -> Result<RsaKeyPair> {
    let mut lines = pem.lines().map(str::trim).filter(|l| !l.is_empty());
    let header = lines.next().context("the GitHub App key is empty")?;
    let body: String = lines.take_while(|l| !l.starts_with("-----")).collect();
    let der = STANDARD
        .decode(body)
        .ok()
        .context("the GitHub App key is not valid PEM")?;
    let parsed = if header == format!("-----BEGIN {PKCS1_LABEL}-----") {
        RsaKeyPair::from_der(&der)
    } else if header == format!("-----BEGIN {PKCS8_LABEL}-----") {
        RsaKeyPair::from_pkcs8(&der)
    } else {
        bail!("the GitHub App key is not an RSA private key");
    };
    parsed.ok().context("the GitHub App key could not be read")
}

/// An RS256 token that signs in as the App itself, not as an installation.
fn app_jwt(pem: &str, client_id: &str, now: u64) -> Result<String> {
    let key = key_pair(pem)?;
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = json!({
        "iat": now.saturating_sub(JWT_BACKDATE_SECS),
        "exp": now + JWT_LIFETIME_SECS,
        "iss": client_id,
    });
    let claims = URL_SAFE_NO_PAD.encode(claims.to_string());
    let message = format!("{header}.{claims}");
    let mut signature = vec![0u8; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        message.as_bytes(),
        &mut signature,
    )
    .ok()
    .context("signing with the GitHub App key failed")?;
    Ok(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// What GitHub returns for a manifest code. No `Debug`: it carries the private key.
#[derive(Deserialize)]
struct Conversion {
    id: u64,
    client_id: String,
    slug: String,
    pem: String,
    owner: ConversionOwner,
}

#[derive(Deserialize)]
struct ConversionOwner {
    login: String,
    id: u64,
}

fn valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_CODE_CHARS
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A clock GitHub disagrees with is the phone's problem, not a deleted App.
fn is_clock_refusal(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["'exp'", "'iat'", "claim", "expiration time"]
        .iter()
        .any(|hint| lower.contains(hint))
}

/// Why no token: said plainly, so callers can tell the owner what to do and never retry.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenRefused {
    #[error("GitHub is not connected")]
    NotConnected,
    #[error("GitHub no longer accepts HQ's App. Reconnect GitHub")]
    Reconnect,
    #[error("HQ's GitHub App is not installed. Install it from Connect GitHub")]
    NotInstalled,
    #[error("HQ's GitHub App does not cover {0}")]
    NotCovered(String),
}

/// GitHub's App API, with the App key. Sends only to the base it was built with, and follows no
/// redirect, so neither the key's tokens nor the JWT go anywhere else.
pub struct AppClient {
    store: Store,
    base: String,
    client: reqwest::Client,
}

impl AppClient {
    pub fn standard() -> Result<Self> {
        Self::new(Store::standard(), API_HOST_URL)
    }

    pub fn new(store: Store, base: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            store,
            base: base.to_string(),
            client,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Trades GitHub's one-time code for the new App and stores it. The caller has already
    /// used up the matching `state` with [`Flows::take`].
    pub async fn exchange(&self, code: &str) -> Result<AppRecord> {
        if !valid_code(code) {
            bail!("that is not a code GitHub issues");
        }
        let url = format!("{}app-manifests/{code}/conversions", self.base);
        let resp = self
            .client
            .post(&url)
            .header("Accept", ACCEPT_JSON)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            bail!("GitHub refused the code with {status}; start again");
        }
        let made: Conversion = resp
            .json()
            .await
            .context("GitHub's answer was not readable")?;
        if !valid_slug(&made.slug) {
            bail!("GitHub's answer named an App HQ cannot link to");
        }
        key_pair(&made.pem)?;
        let record = AppRecord {
            app_id: made.id,
            owner_name: self.public_name(&made.owner.login).await,
            client_id: made.client_id,
            slug: made.slug,
            owner_login: made.owner.login,
            owner_id: made.owner.id,
            installation_id: None,
            repository_selection: None,
            needs_reconnect: false,
        };
        self.store.save_key(&made.pem)?;
        self.store.save_record(&record)?;
        Ok(record)
    }

    /// The person's public display name for commits; the login stands in when there is none.
    async fn public_name(&self, login: &str) -> Option<String> {
        let url = format!("{}users/{login}", self.base);
        let resp = self
            .client
            .get(&url)
            .header("Accept", ACCEPT_JSON)
            .send()
            .await
            .ok()?;
        let value: Value = resp.json().await.ok()?;
        let name = value.get("name")?.as_str()?.trim();
        (!name.is_empty()).then(|| name.to_string())
    }

    fn connected(&self) -> Result<AppRecord> {
        let record = self.store.load().ok_or(TokenRefused::NotConnected)?;
        if record.needs_reconnect {
            return Err(TokenRefused::Reconnect.into());
        }
        Ok(record)
    }

    async fn as_app(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value)> {
        let record = self.connected()?;
        let jwt = app_jwt(&self.store.key()?, &record.client_id, now_secs())?;
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("Accept", ACCEPT_JSON)
            .bearer_auth(jwt);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let value: Value = resp.json().await.unwrap_or(Value::Null);
        if status == 401 {
            self.refused_key(&record, &value)?;
        }
        Ok((status, value))
    }

    /// A 401 marks the connection dead and stops every later try, unless the clock is why.
    fn refused_key(&self, record: &AppRecord, reply: &Value) -> Result<()> {
        let said = reply.get("message").and_then(Value::as_str).unwrap_or("");
        if is_clock_refusal(said) {
            bail!("GitHub refused this device's clock; check the date and time");
        }
        let mut dead = record.clone();
        dead.needs_reconnect = true;
        self.store.save_record(&dead)?;
        Err(TokenRefused::Reconnect.into())
    }

    /// Finds the App's installation through GitHub, never from the browser's return address,
    /// which anyone could have written. Records it, and clears it when there is none.
    pub async fn refresh_installation(&self) -> Result<AppRecord> {
        let (status, value) = self.as_app(Method::GET, "app/installations", None).await?;
        if status != 200 {
            bail!("GitHub would not list the App's installations ({status})");
        }
        let mut record = self.connected()?;
        let found = value.as_array().and_then(|all| {
            all.iter().find(|i| {
                i.pointer("/account/login")
                    .and_then(Value::as_str)
                    .is_some_and(|l| l.eq_ignore_ascii_case(&record.owner_login))
            })
        });
        record.installation_id = found.and_then(|i| i.get("id")).and_then(Value::as_u64);
        record.repository_selection = found
            .and_then(|i| i.get("repository_selection"))
            .and_then(Value::as_str)
            .map(str::to_string);
        self.store.save_record(&record)?;
        Ok(record)
    }

    /// A token for one repository and one step, valid for an hour. `Ok` values never leave
    /// memory: callers send them only to GitHub.
    pub async fn token(&self, repo: &str, step: Step) -> Result<AppToken> {
        let mut record = self.connected()?;
        if record.installation_id.is_none() {
            record = self.refresh_installation().await?;
        }
        let id = record.installation_id.ok_or(TokenRefused::NotInstalled)?;
        let (owner, name) = repo.split_once('/').context("expected owner/name")?;
        if !owner.eq_ignore_ascii_case(&record.owner_login) {
            return Err(TokenRefused::NotCovered(repo.to_string()).into());
        }
        let body = json!({"repositories": [name], "permissions": step.permissions()});
        let path = format!("app/installations/{id}/access_tokens");
        let (status, value) = self.as_app(Method::POST, &path, Some(body)).await?;
        match status {
            201 => value
                .get("token")
                .and_then(Value::as_str)
                .map(|t| AppToken(t.to_string()))
                .context("GitHub's answer held no token"),
            404 => {
                record.installation_id = None;
                self.store.save_record(&record)?;
                Err(TokenRefused::NotInstalled.into())
            }
            422 => Err(TokenRefused::NotCovered(repo.to_string()).into()),
            other => bail!("GitHub would not give a token ({other})"),
        }
    }

    /// Handles GitHub's return: the `state` is used up first, so a code can never arrive
    /// without a flow HQ started, and a replay finds nothing.
    pub async fn finish(&self, flows: &Flows, state: &str, code: &str) -> Result<AppRecord> {
        if !flows.take(state) {
            bail!("this connection was not started here, or has expired; start again");
        }
        self.exchange(code).await
    }

    /// Forgets the App here and returns the page where the person deletes it on GitHub, which
    /// is what cuts its access.
    pub fn disconnect(&self) -> Result<Option<String>> {
        let url = self.store.load().map(|r| r.settings_url());
        self.store.delete()?;
        Ok(url)
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
