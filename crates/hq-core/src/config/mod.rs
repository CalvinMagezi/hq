mod agent;
mod backends;
mod collaboration;
pub mod company;
mod decisions;
mod copilot_usage;
mod disk_watchdog;
mod governance;
mod harness;
mod herdr;
mod instance;
mod llm;
mod memory;
pub mod model_switch;
mod relay;
mod self_update;

pub use agent::*;
pub use backends::{BACKENDS_SCHEMA_VERSION, BackendEntry, BackendKind, BackendsConfig, WireApi};
pub use collaboration::*;
pub use company::{
    CompanyConfig, CompanyIdentity, ConnectorBinding, ListenerDef, company_by_id,
};
pub use decisions::{
    DecisionMode, DecisionRoute, DecisionSite, DecisionsConfig, SITE_EMAIL_FYI, SITE_MEMORY_TURN,
    SITE_NOTIFY_GATE, SITE_TASK_PLACEMENT,
};
pub use copilot_usage::{CopilotUsageConfig, copilot_active};
pub use disk_watchdog::DiskWatchdogConfig;
pub use governance::*;
pub use harness::*;
pub use herdr::{
    HarnessProfileConfig, HerdrConfig, HerdrHostConfig, HostKind, SandboxConfig, SandboxMode, LOCAL_HOST, NATIVE_HOST, native_host_dir, MAX_LAUNCH_BOUND_SECS,
    MIN_LAUNCH_BOUND_SECS,
};
pub use instance::*;
pub use llm::*;
pub use memory::*;
pub use relay::*;
pub use self_update::SelfUpdateConfig;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Yaml},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Shared helper used by multiple sub-modules via `use super::default_true`.
pub(crate) fn default_true() -> bool {
    true
}

impl std::fmt::Debug for HqConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::redact::fmt_redacted(self, "HqConfig", f)
    }
}

impl std::fmt::Debug for RemoteMcpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::redact::fmt_redacted(self, "RemoteMcpServer", f)
    }
}

/// Top-level HQ configuration, loaded from config file + env vars.
/// `Debug` is hand-written so it never prints an API key or token.
#[derive(Clone, Serialize, Deserialize)]
pub struct HqConfig {
    /// Path to the vault directory
    pub vault_path: PathBuf,

    /// OpenRouter API key
    pub openrouter_api_key: Option<String>,

    /// Anthropic API key (direct)
    pub anthropic_api_key: Option<String>,

    /// Google AI API key (direct)
    pub google_ai_api_key: Option<String>,

    /// Cerebras API key - free tier, ~1M tokens/day
    pub cerebras_api_key: Option<String>,

    /// Groq API key - free tier, ~500K tokens/day
    pub groq_api_key: Option<String>,

    /// DeepSeek API key - budget tier, ~$0.14/M input tokens
    pub deepseek_api_key: Option<String>,

    /// OpenAI API key - ChatGPT subscription or API pay
    pub openai_api_key: Option<String>,

    /// Kimi Code subscription API key (console-issued, prefixed `sk-kimi-`,
    /// billed against the kimi.com/code subscription quota rather than
    /// pay-per-token) — distinct from the legacy pay-per-token Moonshot
    /// platform key already read from `KIMI_API_KEY`.
    pub kimi_code_api_key: Option<String>,

    /// Base URL of a self-hosted SearxNG instance (see
    /// `scripts/setup-searxng.sh`). Optional: when set it is tried before the
    /// built-in search engine. Unset by default, since `web_search` works
    /// without it.
    pub searxng_url: Option<String>,

    /// Built-in keyless meta-search (Google, DuckDuckGo, Brave, Wikipedia, Bing News,
    /// arXiv and others, queried in-process). On by default; set `false` to use only
    /// SearxNG and Brave.
    #[serde(default = "default_true")]
    pub web_search_native: bool,

    /// Brave Search API key — paid `web_search` fallback used when SearxNG
    /// is unset or unreachable. https://api.search.brave.com
    pub brave_api_key: Option<String>,

    /// Default LLM model for agent tasks
    #[serde(default = "default_model")]
    pub default_model: String,

    /// When true, only local providers (Ollama, TurboQuant) are loaded by the LLM router.
    /// Cloud providers (Cerebras, Groq, DeepSeek, etc.) are skipped even if API keys are set.
    /// Defaults to false: a fresh install with no local Ollama should route on whatever
    /// cloud keys are actually configured, not silently assume a local model is present.
    #[serde(default)]
    pub local_only: bool,

    /// WebSocket server port for web UI
    #[serde(default = "default_ws_port")]
    pub ws_port: u16,

    /// Max wall-clock seconds for a single chat turn (native session or external
    /// harness). Long agentic tasks need hours, not minutes; default is 6 hours.
    /// Turns can always be stopped early from the chat UI.
    #[serde(default = "default_chat_turn_timeout_secs")]
    pub chat_turn_timeout_secs: u64,

    /// Bind address for the web UI server. "127.0.0.1" = localhost only. Any
    /// non-loopback address (for example "0.0.0.0") needs `web_auth_token`,
    /// or the web server won't start; Tailscale serve works with 127.0.0.1.
    #[serde(default = "default_web_bind")]
    pub web_bind: String,

    /// Shared secret for `/ws` and `/api/*`. Required to bind `web_bind` to a
    /// non-loopback address; clients send it as `Authorization: Bearer`, and
    /// the chat socket uses a short-lived ticket minted with it. URL tokens are
    /// refused. Env: `HQ_WEB_AUTH_TOKEN`.
    #[serde(default)]
    pub web_auth_token: Option<String>,

    /// Browser origins (for example "https://hq.example.ts.net:8443") allowed to
    /// call the API and open the chat socket, besides loopback origins and the
    /// page's own origin. Their hostnames are also accepted as the `Host` of an
    /// unauthenticated instance, which is what stops DNS rebinding.
    #[serde(default)]
    pub web_allowed_origins: Vec<String>,

    /// Directory holding the built PWA (`apps/hq-web/dist/client`), served for
    /// every non-API path. Unset: `web/dist` next to the vault. Env:
    /// `HQ_WEB_STATIC_DIR`.
    #[serde(default)]
    pub web_static_dir: Option<std::path::PathBuf>,

    /// Value for the `HTTP-Referer` header sent to OpenAI-compatible LLM
    /// endpoints (OpenRouter uses it for app attribution). Unset: the header is
    /// omitted. Env: `HQ_HTTP_REFERER`.
    #[serde(default)]
    pub http_referer: Option<String>,

    /// Relay configuration
    #[serde(default)]
    pub relay: RelayConfig,

    /// Agent configuration
    #[serde(default)]
    pub agent: AgentConfig,

    /// Daemon configuration
    #[serde(default)]
    pub daemon: DaemonConfig,

    /// Collaboration configuration (multi-agent bulletin board, curiosity engine)
    #[serde(default)]
    pub collaboration: CollaborationConfig,

    /// Dream engine configuration (tiered memory consolidation)
    #[serde(default)]
    pub dream: DreamConfig,

    /// Instance configuration (local vs cloud, feature flags)
    #[serde(default)]
    pub instance: InstanceConfig,

    /// LLM provider configurations (direct API access to DeepSeek, Novita, etc.)
    ///
    /// Legacy flat provider list. Superseded by [`backends`](Self::backends)
    /// (versioned primary + ordered fallback chain), but retained for backward
    /// compatibility and the adaptive-router legacy path.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,

    /// Versioned, provider-centric backend configuration: an explicit primary
    /// backend plus an ordered fallback chain over named backend entries.
    ///
    /// Defaults to empty; when unset, provider construction falls back to the
    /// legacy [`providers`](Self::providers) list and `*_api_key` fields.
    #[serde(default)]
    pub backends: BackendsConfig,

    /// Self-update lifecycle (branch + test + reinstall of HQ's own binary)
    #[serde(default)]
    pub self_update: SelfUpdateConfig,

    /// Disk/build-artifact threshold monitoring (ported from OpenClaw's
    /// disk-watchdog.sh)
    #[serde(default)]
    pub disk_watchdog: DiskWatchdogConfig,

    /// Copilot credit sampling for the burn-rate meter.
    #[serde(default)]
    pub copilot_usage: CopilotUsageConfig,

    /// Budget configuration for LLM spending
    #[serde(default)]
    pub budget: BudgetConfig,

    /// Where coding-agent sessions run (Herdr, on this machine or over SSH).
    #[serde(default)]
    pub herdr: HerdrConfig,

    /// GitHub Copilot CLI (`gh copilot`) headless harness settings.
    #[serde(default)]
    pub github_copilot: GitHubCopilotConfig,

    /// Governance configuration (autonomy thresholds + agent self-review).
    #[serde(default)]
    pub governance: GovernanceConfig,

    /// Remote MCP servers bridged into HQ's tool registry.
    #[serde(default)]
    pub remote_mcp: Vec<RemoteMcpServer>,

    /// Fast structured-decision model used to gate work before generative calls.
    #[serde(default)]
    pub decisions: DecisionsConfig,

    /// Company configurations for multi-tenant operation.
    #[serde(default)]
    pub companies: Vec<CompanyConfig>,

    /// Fallback company_id when a request carries no company context.
    #[serde(default)]
    pub default_company: String,
}

/// A remote Streamable HTTP MCP server, exposed to agents as
/// `<name>_discover` and `<name>_call`.
///
/// ```yaml
/// remote_mcp:
///   - name: diagrams
///     url: https://example.com/api/mcp
///     api_key: "..."
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct RemoteMcpServer {
    /// Tool-name prefix, e.g. `diagrams` gives `diagrams_discover`.
    pub name: String,
    pub url: String,
    /// Sent as `Authorization: Bearer <api_key>` when set.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Keep the tools out of sessions not driven by a live user turn. On by
    /// default because a remote server's actions can have real-world effects.
    #[serde(default = "default_true")]
    pub live_user_turn_only: bool,
}

fn default_model() -> String {
    "relay".to_string()
}

fn default_ws_port() -> u16 {
    5678
}

fn default_chat_turn_timeout_secs() -> u64 {
    21_600
}

fn default_web_bind() -> String {
    "127.0.0.1".to_string()
}

/// Where `deploy/hq.service` keeps the VPS config and vault. A Herdr pane
/// inherits `herdr.service`'s environment, not the daemon's, so without this
/// fallback `hq chat` in a pane silently loads defaults with no backends.
const SERVER_CONFIG_PATH: &str = "/opt/hq/config.yaml";
const SERVER_VAULT_PATH: &str = "/opt/hq/.vault";

fn home_vault_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".vault")
}

fn default_vault_path() -> PathBuf {
    home_vault_path()
}

/// The per-user path when it exists, else the server path when it exists,
/// else the per-user path (so a fresh machine still gets the usual default).
fn prefer_server_path(user_path: PathBuf, server_path: &std::path::Path) -> PathBuf {
    if !user_path.exists() && readable(server_path) {
        return server_path.to_path_buf();
    }
    user_path
}

/// Existing and openable by this user. A server file that exists but belongs to another account
/// (a CI runner or a second login on the VPS) must not replace the default: reading it would fail.
fn readable(path: &std::path::Path) -> bool {
    if path.is_dir() {
        std::fs::read_dir(path).is_ok()
    } else {
        std::fs::File::open(path).is_ok()
    }
}

/// An explicit path always wins; otherwise the server path only stands in for a missing per-user one.
fn read_path(explicit: Option<PathBuf>, user: PathBuf, server: &std::path::Path) -> PathBuf {
    explicit.unwrap_or_else(|| prefer_server_path(user, server))
}

impl Default for HqConfig {
    fn default() -> Self {
        Self {
            vault_path: default_vault_path(),
            openrouter_api_key: None,
            anthropic_api_key: None,
            google_ai_api_key: None,
            cerebras_api_key: None,
            groq_api_key: None,
            deepseek_api_key: None,
            openai_api_key: None,
            kimi_code_api_key: None,
            searxng_url: None,
            web_search_native: true,
            brave_api_key: None,
            default_model: default_model(),
            local_only: false,
            ws_port: default_ws_port(),
            chat_turn_timeout_secs: default_chat_turn_timeout_secs(),
            web_bind: default_web_bind(),
            web_auth_token: None,
            web_allowed_origins: Vec::new(),
            web_static_dir: None,
            http_referer: None,
            relay: RelayConfig::default(),
            agent: AgentConfig::default(),
            daemon: DaemonConfig::default(),
            collaboration: CollaborationConfig::default(),
            dream: DreamConfig::default(),
            instance: InstanceConfig::default(),
            providers: Vec::new(),
            backends: BackendsConfig::default(),
            self_update: SelfUpdateConfig::default(),
            disk_watchdog: DiskWatchdogConfig::default(),
            copilot_usage: CopilotUsageConfig::default(),
            herdr: HerdrConfig::default(),
            budget: BudgetConfig::default(),
            github_copilot: GitHubCopilotConfig::default(),
            governance: GovernanceConfig::default(),
            remote_mcp: Vec::new(),
            decisions: DecisionsConfig::default(),
            companies: Vec::new(),
            default_company: String::new(),
        }
    }
}

/// Bare shape used to read `instance.instance_type` from the user's own
/// config sources only (file + env), before any compiled struct defaults
/// are in the mix. `HqConfig::load()`'s real Figment chain seeds its base
/// layer from `HqConfig::default()`, which already populates every
/// `instance.features.*` key — so by the time that chain is merged, there
/// is no way to tell "the user never mentioned `features`" from "the user
/// explicitly kept every feature at its `Local` default". Probing the raw
/// sources first is what lets `load()` pick the right base
/// (`InstanceFeatures::cloud()` vs `::default()`) before that ambiguity is
/// introduced.
#[derive(Debug, Default, Deserialize)]
struct InstanceTypeProbe {
    #[serde(default)]
    instance: InstanceTypeProbeInner,
}

#[derive(Debug, Default, Deserialize)]
struct InstanceTypeProbeInner {
    #[serde(default)]
    instance_type: InstanceType,
}

static HTTP_REFERER: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

fn set_http_referer(value: Option<String>) {
    let value = value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if let Ok(mut slot) = HTTP_REFERER.write() {
        *slot = value;
    }
}

/// The configured `HTTP-Referer` for LLM requests, as of the last config load.
/// `None` means the header must be omitted.
pub fn http_referer() -> Option<String> {
    HTTP_REFERER.read().ok().and_then(|slot| slot.clone())
}

impl HqConfig {
    /// Load config from: defaults → config file → env vars. A machine with no
    /// per-user vault but the server's falls back to the server vault, as it
    /// does for the config file; `Default` and `load_from_path` do not, so
    /// tests do not depend on what is installed under `/opt/hq`.
    pub fn load() -> anyhow::Result<Self> {
        let mut config = Self::load_from_path(&Self::config_read_path())?;
        let home_vault = home_vault_path();
        if config.vault_path == home_vault {
            config.vault_path =
                prefer_server_path(home_vault, std::path::Path::new(SERVER_VAULT_PATH));
        }
        Ok(config)
    }

    /// The defaults → config-file layers of the merge chain, without the
    /// final env-var layer. Shared by `load_from_path()` (which adds env on
    /// top) and `load_file_layer_only()` (which doesn't) — the latter is
    /// what `save()` rewrites from, so an env-supplied secret never gets
    /// written back into the committed-shaped YAML file.
    fn file_layer_figment(config_path: &std::path::Path) -> Figment {
        let mut probe = Figment::new();
        if config_path.exists() {
            probe = probe.merge(Yaml::file(config_path));
        }
        probe = probe.merge(Env::prefixed("HQ_").split("__"));
        let instance_type = probe
            .extract::<InstanceTypeProbe>()
            .map(|p| p.instance.instance_type)
            .unwrap_or_default();

        let mut base = HqConfig::default();
        base.instance.instance_type = instance_type.clone();
        base.instance.features = match instance_type {
            InstanceType::Cloud => InstanceFeatures::cloud(),
            InstanceType::Local => InstanceFeatures::default(),
        };

        let mut figment = Figment::from(Serialized::defaults(base));
        if config_path.exists() {
            figment = figment.merge(Yaml::file(config_path));
        }
        figment
    }

    /// Whether any LLM provider key is available, from the config or from the conventional
    /// provider environment variables the router also reads at runtime. Display and
    /// diagnostics only: environment keys are never written back into `config.yaml`.
    pub fn has_llm_key(&self) -> bool {
        self.has_llm_key_with(|name| std::env::var(name).ok())
    }

    fn has_llm_key_with(&self, env: impl Fn(&str) -> Option<String>) -> bool {
        let set = |k: &Option<String>| k.as_deref().is_some_and(|k| !k.trim().is_empty());
        let from_env = |names: &[&str]| {
            names
                .iter()
                .any(|n| env(n).is_some_and(|v| !v.trim().is_empty()))
        };
        set(&self.openrouter_api_key)
            || set(&self.anthropic_api_key)
            || set(&self.google_ai_api_key)
            || !self.providers.is_empty()
            || from_env(&["OPENROUTER_API_KEY", "ANTHROPIC_API_KEY", "GOOGLE_AI_API_KEY", "GEMINI_API_KEY"])
    }

    /// The guts of `load()`, parameterized on the config file path so it's
    /// testable without touching `HQ_CONFIG_PATH`/`~/.hq/config.yaml`.
    pub fn load_from_path(config_path: &std::path::Path) -> anyhow::Result<Self> {
        let figment = Self::file_layer_figment(config_path)
            // HQ_VAULT_PATH, HQ_OPENROUTER_API_KEY, HQ_RELAY__DISCORD_TOKEN, etc.
            // `.split("__")` maps a double underscore to nesting, so secrets on
            // nested config sections (relay/remote_mcp) can be
            // set via env instead of committed-shaped plaintext YAML.
            .merge(Env::prefixed("HQ_").split("__"));

        let config: HqConfig = figment.extract()?;
        set_http_referer(config.http_referer.clone());
        Self::warn_unrecognized_top_level_keys(config_path);
        Ok(config)
    }

    /// Figment silently ignores config.yaml keys that don't match any
    /// `HqConfig` field (e.g. a stray `get: "clickup"` typo found in a real
    /// deployment) — this doesn't block loading, but logs a warning so a
    /// typo doesn't go unnoticed. Compares against `HqConfig::default()`'s
    /// own serialized field set rather than a hand-maintained list, so it
    /// can't drift from the struct.
    fn warn_unrecognized_top_level_keys(config_path: &std::path::Path) {
        let Ok(raw) = std::fs::read_to_string(config_path) else {
            return;
        };
        let Ok(serde_yaml::Value::Mapping(user_keys)) = serde_yaml::from_str(&raw) else {
            return;
        };
        let Ok(serde_yaml::Value::Mapping(known_keys)) = serde_yaml::to_value(HqConfig::default())
        else {
            return;
        };
        for key in user_keys.keys() {
            if !known_keys.contains_key(key) {
                tracing::warn!(
                    key = key.as_str().unwrap_or("?"),
                    path = %config_path.display(),
                    "config.yaml has an unrecognized top-level key (ignored — likely a typo)"
                );
            }
        }
    }

    /// When any cloud API key is present, turn off `local_only` (so the
    /// router actually uses the key instead of assuming a local Ollama
    /// instance) and bump `default_model` off a stale `ollama/*` tag onto
    /// the generic `"relay"` router alias. Shared by `hq onboard` and
    /// `hq env` so entering a cloud key behaves identically from either
    /// command.
    pub fn apply_cloud_key_flip(&mut self, has_any_cloud_key: bool) {
        if has_any_cloud_key {
            self.local_only = false;
            if self.default_model.starts_with("ollama/") {
                self.default_model = "relay".to_string();
            }
        }
    }

    /// Apply `patch` to the config-file's own layer (defaults + whatever is
    /// actually written in the file — deliberately *not* the env-merged
    /// config a running process is using) and write the result back,
    /// creating the parent directory if needed.
    ///
    /// Writing the whole typed struct (rather than hand-building a YAML
    /// string field by field) round-trips every section —
    /// relay/remote_mcp/companies/etc. — instead of silently dropping
    /// whatever a partial writer didn't know about. Rebuilding from the
    /// file layer rather than `HqConfig::load()`'s env-merged result is
    /// what keeps this from writing an env-supplied secret
    /// (`HQ_DEEPSEEK_API_KEY`, `HQ_RELAY__DISCORD_TOKEN`, ...) into the
    /// plaintext file — the whole point of setting it via env in the first
    /// place.
    pub fn save_patch(patch: impl FnOnce(&mut HqConfig)) -> anyhow::Result<HqConfig> {
        Self::save_patch_to_path(&Self::config_file_path(), patch)
    }

    /// The guts of `save_patch()`, parameterized on the config file path so
    /// it's testable without touching `HQ_CONFIG_PATH`/`~/.hq/config.yaml`.
    fn save_patch_to_path(
        config_path: &std::path::Path,
        patch: impl FnOnce(&mut HqConfig),
    ) -> anyhow::Result<HqConfig> {
        let mut updated: HqConfig = Self::file_layer_figment(config_path).extract()?;
        patch(&mut updated);
        if let Some(parent) = config_path.parent() {
            crate::fs_private::create_private_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(&updated)?;
        crate::fs_private::write_private(config_path, yaml)?;
        Ok(updated)
    }

    /// Path every writer (`save_patch`, `set_key`, `hq config`, onboarding)
    /// uses: `HQ_CONFIG_PATH`, else `~/.hq/config.yaml`. Never the server
    /// fallback, so a shell on a server cannot silently edit the daemon's config.
    pub fn config_file_path() -> PathBuf {
        match std::env::var("HQ_CONFIG_PATH") {
            Ok(path) => PathBuf::from(path),
            Err(_) => Self::hq_dir().join("config.yaml"),
        }
    }

    /// Path `load()` reads: as `config_file_path`, except that with no explicit
    /// path and no per-user file the VPS deploy path `/opt/hq/config.yaml` is
    /// used when it exists (a Herdr pane inherits no `HQ_CONFIG_PATH`).
    /// Readers that must see the file in force (doctor, the secret-file
    /// guard) use this; writers must not.
    pub fn config_read_path() -> PathBuf {
        read_path(
            std::env::var_os("HQ_CONFIG_PATH").map(PathBuf::from),
            Self::hq_dir().join("config.yaml"),
            std::path::Path::new(SERVER_CONFIG_PATH),
        )
    }

    /// HQ data directory: ~/.hq/
    pub fn hq_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".hq")
    }

    /// Installed binary path. Checks `HQ_BIN_PATH` first, falls back to the
    /// macOS install convention (`/usr/local/bin/hq`) — override this on Linux
    /// installs where that path isn't the norm.
    pub fn bin_path() -> PathBuf {
        std::env::var("HQ_BIN_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/usr/local/bin/hq"))
    }

    /// Path to the SQLite database
    pub fn db_path(&self) -> PathBuf {
        self.vault_path.join("_data").join("vault.db")
    }

    /// Write a top-level key = value into the config YAML file.
    /// Creates the file if it doesn't exist. Simple line-based patch — works
    /// for scalar string/bool/int fields at the top level.
    pub fn set_key(key: &str, value: &str) -> anyhow::Result<()> {
        let config_path = Self::config_file_path();
        let mut content = if config_path.exists() {
            std::fs::read_to_string(&config_path)?
        } else {
            String::new()
        };

        let pattern = format!("{key}: ");
        let new_line = format!("{key}: \"{value}\"");

        if let Some(pos) = content.find(&pattern) {
            let line_end = content[pos..]
                .find('\n')
                .map(|i| pos + i)
                .unwrap_or(content.len());
            content.replace_range(pos..line_end, &new_line);
        } else {
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&new_line);
            content.push('\n');
        }

        if let Some(parent) = config_path.parent() {
            crate::fs_private::create_private_dir_all(parent)?;
        }
        crate::fs_private::write_private(&config_path, content)?;
        Ok(())
    }

    pub fn company_by_id(&self, id: &str) -> Option<&CompanyConfig> {
        company_by_id(&self.companies, id)
    }

    pub fn default_company_config(&self) -> Option<&CompanyConfig> {
        if !self.default_company.is_empty() {
            self.company_by_id(&self.default_company)
        } else {
            self.companies.first()
        }
    }

}

/// Resolve the model a fresh session will actually start on.
///
/// The backends chain is the source of truth: when configured, the primary
/// backend's model drives turns (e.g. copilot → `claude-sonnet-5`), so callers
/// must lead with it, never the legacy `default_model`, which only applies
/// when no chain exists. Order: chain primary model → explicit `relay.model`
/// override → `default_model`.
///
/// Shared by hq-relay's session-reset banner and hq-agent's `SessionConfig`
/// resolution so both surfaces agree on what a session actually runs on.
pub fn resolve_session_model(config: &HqConfig) -> String {
    if config.backends.is_configured()
        && let Some(entry) = config.backends.backend(&config.backends.primary)
        && let Some(m) = entry
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
    {
        return m.to_string();
    }
    config
        .relay
        .model
        .clone()
        .filter(|m| !m.trim().is_empty() && m.trim() != "relay")
        .unwrap_or_else(|| config.default_model.clone())
}

#[cfg(test)]
mod tests;
