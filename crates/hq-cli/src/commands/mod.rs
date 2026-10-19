pub mod agents;
pub mod chat;
pub mod clean;
pub mod config;
pub mod content;
pub mod copilot;
pub mod cursor_mcp_config;
pub mod daemon;
pub mod decisions;
pub mod doctor;
pub mod env;
pub mod health;
#[cfg(unix)]
pub mod host;
#[cfg(unix)]
pub mod host_install;
/// Off Unix there is no coding-agent host; the command exists so `hq host` can say so.
#[cfg(not(unix))]
pub mod host {
    use anyhow::{Result, bail};
    use std::path::PathBuf;

    pub struct HostArgs {
        pub sub: String,
        pub arg: Option<String>,
        pub addr: Option<String>,
        pub dir: Option<PathBuf>,
        pub allow_unsandboxed: bool,
        pub max_agents: Option<usize>,
        pub max_starts: Option<usize>,
        pub key: Option<String>,
        pub gate_key: Option<String>,
        pub from: Option<String>,
        pub port: Option<u16>,
        pub unix: Option<PathBuf>,
        pub rest: Vec<String>,
    }

    pub async fn run(_args: HostArgs) -> Result<()> {
        bail!(
            "the coding-agent host runs on Linux, macOS and WSL2, not in native Windows (HQ Lite). \
             Use Full HQ in WSL2 for coding agents."
        )
    }
}
pub mod connector;
pub mod install;
pub mod kill;
pub mod lite;
pub mod logs;
pub mod mailbox;
pub mod mcp;
pub mod mcp_serve;
pub mod memory;
pub mod models;
pub mod net;
pub mod notify_restart;
pub mod onboard;
pub mod pair;
pub mod profile;
pub mod ps;
pub mod queue;
pub mod restart;
pub mod search;
#[cfg(unix)]
pub mod self_apply;
pub mod web;
#[cfg(not(unix))]
pub mod self_apply {
    use anyhow::{Result, bail};
    use hq_core::config::HqConfig;

    pub async fn run(_config: &HqConfig, _run_id: i64) -> Result<()> {
        bail!("self-apply is not available on Windows")
    }
}
pub mod agent_skill;
pub mod service;
pub mod sessions;
pub mod shortcuts;
pub mod skills;
pub mod skills_evidence;
pub mod stack;
pub mod start;
pub mod status;
pub mod stop;
pub mod task;
pub mod tasks;
pub mod tools;
pub mod update;
pub mod usage;
pub mod vault;
pub mod vault_import;
