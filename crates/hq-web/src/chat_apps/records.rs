//! What the chat apps card shows besides the config: the bot's public identity, remembered
//! from the last token check, and the relay's own status file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Platform;
use super::verify::Verified;

/// Hex characters of the token's SHA-256 kept to tell whether an identity still matches.
const FINGERPRINT_HEX: usize = 16;

/// A bot's public name and ids, tied to the token they were read with by a short hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BotIdentity {
    pub username: String,
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub message_content: Option<bool>,
    token_sha: String,
}

fn identities_path(vault: &Path) -> PathBuf {
    vault.join("_system").join("chat-apps.json")
}

fn fingerprint(token: &str) -> String {
    let digest: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    digest[..FINGERPRINT_HEX].to_string()
}

fn read_all(vault: &Path) -> BTreeMap<String, BotIdentity> {
    std::fs::read_to_string(identities_path(vault))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_all(vault: &Path, all: &BTreeMap<String, BotIdentity>) -> std::io::Result<()> {
    let path = identities_path(vault);
    hq_core::fs_private::create_private_dir_all(path.parent().unwrap_or(vault))?;
    let json = serde_json::to_string_pretty(all).map_err(std::io::Error::other)?;
    hq_core::fs_private::write_private(&path, json)
}

/// The identity remembered for this platform, if it was read with `token`.
pub(crate) fn load_identity(vault: &Path, platform: Platform, token: &str) -> Option<BotIdentity> {
    read_all(vault)
        .remove(platform.as_str())
        .filter(|id| id.token_sha == fingerprint(token))
}

/// Remembers what a token check returned, so the card can show it after a restart.
pub(crate) fn store_identity(vault: &Path, platform: Platform, token: &str, v: &Verified) {
    let mut all = read_all(vault);
    all.insert(
        platform.as_str().to_string(),
        BotIdentity {
            username: v.username.clone(),
            application_id: v.application_id.clone(),
            message_content: v.message_content,
            token_sha: fingerprint(token),
        },
    );
    if let Err(e) = write_all(vault, &all) {
        tracing::warn!(error = %e, "chat apps: could not remember the bot identity");
    }
}

/// Forgets a platform's identity and its relay status, after a disconnect.
pub(crate) fn forget(vault: &Path, platform: Platform) {
    let mut all = read_all(vault);
    if all.remove(platform.as_str()).is_some()
        && let Err(e) = write_all(vault, &all)
    {
        tracing::warn!(error = %e, "chat apps: could not forget the bot identity");
    }
    let _ = std::fs::remove_file(status_path(vault, platform));
}

fn status_path(vault: &Path, platform: Platform) -> PathBuf {
    vault
        .join("_system")
        .join("relay-status")
        .join(format!("{}.json", platform.as_str()))
}

#[derive(Deserialize)]
struct StatusFile {
    status: String,
    last_seen: String,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

/// The relay's status file as JSON. `current` is false when it was written before this
/// process started, so a relay that no longer runs never reads as connected.
pub(crate) fn relay_status(vault: &Path, platform: Platform, started_at: i64) -> Value {
    let Some(file) = std::fs::read_to_string(status_path(vault, platform))
        .ok()
        .and_then(|raw| serde_json::from_str::<StatusFile>(&raw).ok())
    else {
        return Value::Null;
    };
    let seen = chrono::DateTime::parse_from_rfc3339(&file.last_seen)
        .map(|t| t.timestamp())
        .ok();
    json!({
        "status": file.status,
        "last_seen": file.last_seen,
        "detail": file.detail,
        "error": file.error_message,
        "current": seen.is_some_and(|t| t >= started_at),
    })
}
