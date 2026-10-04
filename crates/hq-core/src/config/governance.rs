use serde::{Deserialize, Serialize};

use super::default_true;

/// Governance configuration: agent self-review, skill-write approval, and
/// the bash tool's execution boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceConfig {
    /// Run the post-session background review hook to propose memories/skills.
    #[serde(default = "default_true")]
    pub background_review: bool,
    /// Require human approval before skill writes take effect.
    /// When true, skill_manage writes staged to `<skills_dir>/_proposed/`.
    #[serde(default)]
    pub skills_write_approval: bool,
    /// Environment and sandbox policy for the agent's `bash` tool.
    #[serde(default)]
    pub bash: BashConfig,
}

impl Default for GovernanceConfig {
    fn default() -> Self {
        Self {
            background_review: true,
            skills_write_approval: false,
            bash: BashConfig::default(),
        }
    }
}

/// How the `bash` tool isolates the commands it runs.
///
/// ```yaml
/// governance:
///   bash:
///     env_passthrough: [GH_TOKEN, GITHUB_TOKEN]
///     sandbox: required   # the default; best_effort or off must be chosen explicitly
///     network: true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashConfig {
    /// Secret env vars the operator deliberately hands to bash children, on
    /// top of the fixed non-secret allowlist. Empty by default, so a fresh
    /// install gives bash no credentials at all.
    #[serde(default)]
    pub env_passthrough: Vec<String>,
    /// Whether commands run inside bubblewrap (Linux) or sandbox-exec (macOS).
    #[serde(default)]
    pub sandbox: BashSandboxMode,
    /// Let sandboxed commands reach the network. Only enforced while a
    /// sandbox is active; `false` adds `--unshare-net` / a seatbelt deny.
    #[serde(default = "default_true")]
    pub network: bool,
}

impl Default for BashConfig {
    fn default() -> Self {
        Self {
            env_passthrough: Vec::new(),
            sandbox: BashSandboxMode::default(),
            network: true,
        }
    }
}

/// Sandbox strictness for the `bash` tool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BashSandboxMode {
    /// Never wrap; env allowlist and policy checks only. Explicit opt-out.
    Off,
    /// Wrap when the sandbox tool works here, otherwise run unwrapped. Explicit
    /// opt-in to running model-written shell commands without isolation.
    BestEffort,
    /// Wrap every command; refuse to run any when the sandbox tool is missing
    /// or broken. The default, so a host without bubblewrap or sandbox-exec
    /// gets no model-generated bash until the operator decides otherwise.
    #[default]
    Required,
}
