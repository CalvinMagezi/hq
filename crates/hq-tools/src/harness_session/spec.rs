//! Per-harness session descriptors: how to spawn each CLI interactively, how
//! to resume a prior session, and how to harvest a resume token from output.
//! Configured profiles (`agent_host.harness_profiles`) resolve here too, so every
//! caller that accepts a harness name accepts a profile name.

use anyhow::{Result, bail};
use hq_core::config::{AgentHostConfig, HarnessProfileConfig, HqConfig};

use super::category::HarnessCategory;

/// How a harness resumes a prior session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResumeStrategy {
    /// Replace the spawn args with these; `{token}` is substituted with the
    /// harvested resume token (spawn fails over to fresh if none saved).
    Args(&'static [&'static str]),
    /// Resume a specific conversation when its id is known, else continue the
    /// most recent one in the directory. The id is exact; the fallback can pick
    /// the wrong conversation when two sessions share a directory.
    TokenOrArgs {
        with_token: &'static [&'static str],
        otherwise: &'static [&'static str],
    },
    /// The harness keys sessions off a dedicated directory passed as
    /// `--session-dir <dir>`; re-spawning with the same dir resumes.
    SessionDir,
    /// No resume support wired yet: resume falls back to a fresh spawn.
    None,
}

/// Codex config override that skips its update prompt in sessions HQ starts.
const CODEX_NO_UPDATE_CHECK: &str = "check_for_update_on_startup=false";

#[derive(Debug, Clone)]
pub struct HarnessSessionSpec {
    pub harness: &'static str,
    /// Coding agent or general-purpose agent: decides what HQ hands it.
    pub category: HarnessCategory,
    /// host agent kind (`agent start --kind`). The host resolves the CLI on
    /// the target host, so HQ never needs the binary on its own machine.
    pub kind: &'static str,
    /// Args for a fresh interactive spawn.
    pub args: &'static [&'static str],
    pub resume: ResumeStrategy,
    /// Regex with one capture group extracting a resume token from session output.
    pub token_pattern: Option<&'static str>,
    /// Arguments that run the agent once for a single brief and exit, with
    /// `{brief}` standing for the text: how a stack specialist works a task.
    /// `None` means the harness has no one-shot mode wired.
    pub one_shot: Option<&'static [&'static str]>,
    /// Substring of a workspace-trust dialog whose default answer is "trust".
    /// Only for harnesses verified to default that way: when spawn finds the
    /// agent blocked on it, spawn presses Enter. Any other blocked dialog is
    /// left for the caller to read and answer, since some CLIs now default a
    /// trust prompt to "No, exit".
    pub trust_pattern: Option<&'static str>,
    /// The flag that picks the CLI's model (`--model <id>`), checked against each CLI's `--help`.
    /// `None` means HQ does not know one, so asking for a model fails instead of being ignored.
    pub model_flag: Option<&'static str>,
    /// The flag that hands the CLI its first prompt at start (`<flag>=<prompt>`). Set for a CLI
    /// that keeps redrawing after its first screen (Copilot reloads its MCP servers) and so loses
    /// a prompt typed into it. `None` means the prompt is typed once the screen is up.
    pub prompt_flag: Option<&'static str>,
}

impl HarnessSessionSpec {
    /// The executable the host starts for this kind, as it is found on PATH.
    pub fn binary(&self) -> &'static str {
        match self.kind {
            "cursor" => "cursor-agent",
            other => other,
        }
    }
}

/// How a session talks to its agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// A terminal the host reads and types into.
    Pty,
    /// The Agent Client Protocol over the agent's stdio.
    Acp,
}

const TRANSPORT_ACP: &str = "acp";
const TRANSPORT_PTY: &str = "pty";

/// The arguments that start a built-in harness's ACP server, for the CLIs that have one.
fn builtin_acp_args(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "opencode" | "agy" | "cursor" | "kimi" | "hermes" => Some(&["acp"]),
        "github-copilot" | "copilot" => Some(&["--acp", "--stdio"]),
        _ => None,
    }
}

/// Copilot starts with every tool allowed, as `claude-code` starts with its
/// permission bypass: in manual approval every command waits on a dialog nobody
/// is there to answer. The denials name commands that undo or leak work outside
/// the agent's own change: pushing, stashing, resetting or cleaning a tree another
/// agent may share, deleting files (`git rm` stays allowed, and git can bring back
/// what it removes), printing the environment. A deny wins over
/// `--allow-all-tools`, and Copilot checks each command of a compound line.
///
/// A rule names a command or a command and its subcommand, never a flag: checked
/// on Copilot CLI 1.0.105, `shell(rm -r)` let `rm -r dir` through and `shell(rm)`
/// stopped it. These guard against a slip; what stops a push or a deploy for
/// certain is the agent not holding the credential. A profile with its own `args`
/// replaces all of this.
pub const COPILOT_ARGS: &[&str] = &[
    "--allow-all-tools",
    "--deny-tool=shell(git push)",
    "--deny-tool=shell(git stash)",
    "--deny-tool=shell(git reset)",
    "--deny-tool=shell(git clean)",
    "--deny-tool=shell(rm)",
    "--deny-tool=shell(env)",
    "--deny-tool=shell(printenv)",
];

/// The known interactive-session harnesses.
///
/// Resume flags verified against each CLI where known; harnesses marked
/// `ResumeStrategy::None` need their flag confirmed before wiring
/// (tracked as adapter follow-ups).
pub const SPECS: &[HarnessSessionSpec] = &[
    HarnessSessionSpec {
        harness: "claude-code",
        category: HarnessCategory::Coding,
        kind: "claude",
        // Without the permission bypass every tool call in an unattended
        // session blocks on an approval dialog nobody is there to answer.
        args: &["--dangerously-skip-permissions"],
        // `claude --resume <id>` is exact; `claude -c` continues the most recent
        // session in the cwd, used until the id is known.
        resume: ResumeStrategy::TokenOrArgs {
            with_token: &["--dangerously-skip-permissions", "--resume", "{token}"],
            otherwise: &["--dangerously-skip-permissions", "-c"],
        },
        token_pattern: None,
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "qwen",
        category: HarnessCategory::Coding,
        kind: "qwen",
        args: &["--auth-type", "qwen-oauth"],
        resume: ResumeStrategy::Args(&["--auth-type", "qwen-oauth", "--continue"]),
        token_pattern: None,
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "cursor",
        category: HarnessCategory::Coding,
        kind: "cursor",
        // `--trust` only works in headless mode; the interactive trust dialog is answered at launch.
        args: &["--force"],
        resume: ResumeStrategy::Args(&["--force", "--resume", "{token}"]),
        token_pattern: Some(r"chat[_-]id[:=]\s*([A-Za-z0-9_-]+)"),
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "pi",
        category: HarnessCategory::Coding,
        kind: "pi",
        args: &[],
        resume: ResumeStrategy::SessionDir,
        token_pattern: None,
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "codex",
        category: HarnessCategory::Coding,
        kind: "codex",
        // The background daemon check reads other processes, which the sandbox forbids.
        // The startup update check can draw "Update available!" after the agent first
        // reads as idle, so the brief would land in it and pick "Update now".
        args: &["--no-daemon", "-c", CODEX_NO_UPDATE_CHECK],
        resume: ResumeStrategy::Args(&[
            "--no-daemon",
            "-c",
            CODEX_NO_UPDATE_CHECK,
            "resume",
            "--last",
        ]),
        token_pattern: None,
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "opencode",
        category: HarnessCategory::Coding,
        kind: "opencode",
        args: &[],
        resume: ResumeStrategy::Args(&["--continue"]),
        token_pattern: None,
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "kimi",
        category: HarnessCategory::Coding,
        kind: "kimi",
        args: &[],
        resume: ResumeStrategy::None,
        token_pattern: None,
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "antigravity",
        category: HarnessCategory::Coding,
        kind: "agy",
        args: &["--dangerously-skip-permissions"],
        // `agy -c` continues the most recent conversation in the cwd.
        resume: ResumeStrategy::Args(&["--dangerously-skip-permissions", "-c"]),
        token_pattern: None,
        one_shot: None,
        // agy asks "Do you trust the contents of this project?" on first use
        // of a cwd; default selection is "Yes, I trust this folder".
        trust_pattern: Some("trust the contents"),
        model_flag: Some("--model"),
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "github-copilot",
        category: HarnessCategory::Coding,
        kind: "copilot",
        args: COPILOT_ARGS,
        resume: ResumeStrategy::None,
        token_pattern: None,
        one_shot: None,
        trust_pattern: None,
        model_flag: Some("--model"),
        // `-i/--interactive <prompt>` in `copilot --help` (1.0.100): start interactive and run it.
        prompt_flag: Some("--interactive"),
    },
    // General-purpose agents. Each is launched into its interactive chat the way
    // its own documentation starts it; the resume flags are not verified, so a
    // resume is a fresh start that the agent's own session store may continue.
    // `agent_host.harness_profiles` can replace the command and arguments for
    // an install that differs (a profile may reuse the built-in name).
    HarnessSessionSpec {
        harness: "hermes",
        category: HarnessCategory::GeneralPurpose,
        kind: "hermes",
        // Nous Research's Hermes Agent: `hermes chat` is its interactive loop.
        // On a TTY (a host pane is one) `hermes chat -q` stays open after its
        // answer; `--oneshot` makes it exit, and it skips the first-run
        // telemetry dialog.
        args: &["chat"],
        resume: ResumeStrategy::None,
        token_pattern: None,
        one_shot: Some(&["chat", "--oneshot", "-q", "{brief}"]),
        trust_pattern: None,
        model_flag: None,
        prompt_flag: None,
    },
    HarnessSessionSpec {
        harness: "openclaw",
        category: HarnessCategory::GeneralPurpose,
        kind: "openclaw",
        // OpenClaw's terminal interface; `openclaw agent --message` is the
        // one-turn form its own automations use. Both talk to a gateway on
        // loopback by default, which the sandbox refuses, so `--local` runs the
        // agent in process. OpenClaw refuses `--local` while a gateway runs for
        // the same state directory, so an HQ host does not run that daemon.
        args: &["tui", "--local"],
        resume: ResumeStrategy::None,
        token_pattern: None,
        one_shot: Some(&["agent", "--local", "--message", "{brief}"]),
        trust_pattern: None,
        model_flag: None,
        prompt_flag: None,
    },
];

/// The built-in harnesses of one category, by name.
pub fn names_in(category: HarnessCategory) -> Vec<&'static str> {
    SPECS
        .iter()
        .filter(|s| s.category == category)
        .map(|s| s.harness)
        .collect()
}

/// The category of a harness or profile name, when it resolves. Loads the
/// config; a caller with many names uses `category_in` on one load.
pub fn category_of(name: &str) -> Option<HarnessCategory> {
    category_in(&agent_host_config(), name)
}

/// `category_of` against a config already in hand.
pub fn category_in(cfg: &AgentHostConfig, name: &str) -> Option<HarnessCategory> {
    resolve_in(cfg, name).ok().map(|h| h.spec.category)
}

/// Aliases accepted in addition to each spec's canonical `harness` name
/// (relay/dispatch paths already map "agy"/"jetski" etc.; sessions match).
const ALIASES: &[(&str, &str)] = &[("agy", "antigravity")];

/// Map an alias to its canonical harness name; unknown names pass through.
pub fn canonical_name(harness: &str) -> &str {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == harness)
        .map(|(_, canonical)| *canonical)
        .unwrap_or(harness)
}

pub fn spec_for(harness: &str) -> Option<&'static HarnessSessionSpec> {
    let name = canonical_name(harness);
    SPECS.iter().find(|s| s.harness == name)
}

/// A harness name resolved to the built-in spec that drives it, and the
/// profile layered on top when the name is a configured one.
#[derive(Debug, Clone)]
pub struct Harness {
    /// The name callers used; what the registry stores and resume looks up.
    pub name: String,
    pub spec: &'static HarnessSessionSpec,
    pub profile: Option<HarnessProfileConfig>,
    /// The model the session was asked to run, passed with the spec's `model_flag`.
    pub model: Option<String>,
}

/// Longest model id accepted; real ids are far shorter.
const MAX_MODEL_CHARS: usize = 128;

/// A model id is passed to the CLI as one argument. It may not start with `-`, so it can never
/// read as another flag, and holds only the characters model ids use.
fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= MAX_MODEL_CHARS
        && !model.starts_with('-')
        && model.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '@' | '+')
        })
}

impl Harness {
    pub fn category(&self) -> HarnessCategory {
        self.spec.category
    }

    /// This harness running `model`. Refused when the CLI has no model flag HQ knows, rather
    /// than started on its default with the request silently dropped.
    pub fn with_model(mut self, model: Option<&str>) -> Result<Self> {
        let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) else {
            return Ok(self);
        };
        if self.spec.model_flag.is_none() {
            bail!(
                "'{}' has no model flag HQ knows, so it cannot be started on '{model}'; leave model out to use its own default",
                self.name
            );
        }
        if !valid_model(model) {
            bail!(
                "'{model}' is not a model id (letters, digits and . _ - : / @ + only, not starting with -)"
            );
        }
        self.model = Some(model.to_string());
        Ok(self)
    }

    /// The flag and model to append to the CLI's arguments, empty when no model was asked for.
    pub fn model_args(&self) -> Vec<String> {
        match (self.spec.model_flag, &self.model) {
            (Some(flag), Some(model)) => vec![flag.to_string(), model.clone()],
            _ => Vec::new(),
        }
    }

    /// The first prompt as launch arguments, empty when the CLI takes no prompt at start. The
    /// `flag=value` form keeps a prompt that starts with `-` from reading as another flag.
    pub fn prompt_args(&self, prompt: &str) -> Vec<String> {
        self.spec
            .prompt_flag
            .map(|flag| vec![format!("{flag}={prompt}")])
            .unwrap_or_default()
    }

    /// Arguments for a fresh spawn: the profile's when it sets any.
    pub fn fresh_args(&self) -> Vec<String> {
        match self.profile.as_ref().and_then(|p| p.args.as_ref()) {
            Some(args) => args.clone(),
            None => self.spec.args.iter().map(|a| a.to_string()).collect(),
        }
    }

    /// How this session talks to its agent: a terminal unless the profile asks for ACP.
    pub fn transport(&self) -> Transport {
        let acp = self.profile.as_ref().and_then(|p| p.transport.as_deref()) == Some(TRANSPORT_ACP);
        if acp { Transport::Acp } else { Transport::Pty }
    }

    /// Whether the session is given HQ's MCP connection: an ACP profile that asks for it.
    pub fn gives_hq_mcp(&self) -> bool {
        let asked = self.profile.as_ref().and_then(|p| p.hq_mcp) == Some(true);
        asked && self.transport() == Transport::Acp
    }

    /// The program and arguments that start the agent's ACP server.
    pub fn acp_command(&self) -> Result<Vec<String>> {
        let own = self.profile.as_ref().and_then(|p| p.acp_args.clone());
        let args = match own {
            Some(args) => args,
            None => builtin_acp_args(self.spec.kind)
                .map(|a| a.iter().map(|x| x.to_string()).collect())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "'{}' has no known ACP command; set acp_args on its profile",
                        self.name
                    )
                })?,
        };
        Ok(std::iter::once(self.spec.binary().to_string())
            .chain(args)
            .collect())
    }

    /// Arguments that run the agent once for `brief` and exit: the profile's
    /// `one_shot_args` when set, else the spec's. `None` when neither has one.
    pub fn one_shot_args(&self, brief: &str) -> Option<Vec<String>> {
        let template: Vec<String> =
            match self.profile.as_ref().and_then(|p| p.one_shot_args.as_ref()) {
                Some(args) => args.clone(),
                None => self.spec.one_shot?.iter().map(|a| a.to_string()).collect(),
            };
        Some(
            template
                .iter()
                .map(|a| a.replace("{brief}", brief))
                .collect(),
        )
    }
}

/// Resolve `name` against the profiles in `cfg`, then the built-in harnesses.
/// A profile may share a built-in's name to replace how it launches.
pub fn resolve_in(cfg: &AgentHostConfig, name: &str) -> Result<Harness> {
    if let Some(profile) = cfg.harness_profiles.get(name) {
        check_transport(name, profile)?;
        let Some(spec) = spec_for(&profile.base) else {
            bail!(
                "harness profile '{name}' names unknown base '{}' (known: {})",
                profile.base,
                builtin_names().join(", ")
            );
        };
        return Ok(Harness {
            name: name.to_string(),
            spec,
            profile: Some(profile.clone()),
            model: None,
        });
    }
    match spec_for(name) {
        Some(spec) => Ok(Harness {
            name: spec.harness.to_string(),
            spec,
            profile: None,
            model: None,
        }),
        None => bail!(
            "unknown session harness '{name}' (known: {})",
            known_in(cfg).join(", ")
        ),
    }
}

/// A profile's transport must be one this build knows, and an ACP profile must be able to start
/// an ACP server directly (a wrapper `command` cannot).
fn check_transport(name: &str, profile: &HarnessProfileConfig) -> Result<()> {
    let acp = match profile.transport.as_deref() {
        None | Some(TRANSPORT_PTY) => false,
        Some(TRANSPORT_ACP) => true,
        Some(other) => bail!("harness profile '{name}': transport '{other}' is not 'pty' or 'acp'"),
    };
    if !acp {
        if profile.acp_args.is_some() || profile.hq_mcp.is_some() {
            bail!("harness profile '{name}': acp_args and hq_mcp only apply with transport: acp");
        }
        return Ok(());
    }
    if profile.command.is_some() {
        bail!("harness profile '{name}': a wrapper command cannot be used over ACP");
    }
    if profile.args.is_some() {
        bail!("harness profile '{name}': over ACP the terminal `args` do not apply; use acp_args");
    }
    if profile.acp_args.as_ref().is_some_and(Vec::is_empty) {
        bail!("harness profile '{name}': acp_args is empty, so nothing would start the ACP server");
    }
    Ok(())
}

/// `resolve_in` against the loaded config; without a readable config only the
/// built-in harnesses resolve.
pub fn resolve(name: &str) -> Result<Harness> {
    resolve_in(&agent_host_config(), name)
}

pub fn known_in(cfg: &AgentHostConfig) -> Vec<String> {
    let mut names = builtin_names();
    for profile in cfg.harness_profiles.keys() {
        if !names.contains(profile) {
            names.push(profile.clone());
        }
    }
    names
}

pub fn known_harnesses() -> Vec<String> {
    known_in(&agent_host_config())
}

fn builtin_names() -> Vec<String> {
    SPECS.iter().map(|s| s.harness.to_string()).collect()
}

pub fn agent_host_config() -> AgentHostConfig {
    HqConfig::load().map(|c| c.agent_host).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acp_profile(base: &str, transport: &str) -> AgentHostConfig {
        let mut cfg = AgentHostConfig::default();
        cfg.harness_profiles.insert(
            "p".into(),
            HarnessProfileConfig {
                base: base.into(),
                transport: Some(transport.into()),
                ..Default::default()
            },
        );
        cfg
    }

    #[test]
    fn an_acp_profile_starts_the_acp_server_of_its_base() {
        for (base, want) in [
            ("opencode", vec!["opencode", "acp"]),
            ("github-copilot", vec!["copilot", "--acp", "--stdio"]),
            ("antigravity", vec!["agy", "acp"]),
            ("cursor", vec!["cursor-agent", "acp"]),
        ] {
            let h = resolve_in(&acp_profile(base, "acp"), "p").unwrap();
            assert_eq!(h.transport(), Transport::Acp);
            assert_eq!(h.acp_command().unwrap(), want);
        }
    }

    #[test]
    fn a_profile_without_a_transport_stays_on_the_terminal() {
        let h = resolve_in(&acp_profile("opencode", "pty"), "p").unwrap();
        assert_eq!(h.transport(), Transport::Pty);
        assert_eq!(
            resolve_in(&AgentHostConfig::default(), "opencode")
                .unwrap()
                .transport(),
            Transport::Pty
        );
    }

    #[test]
    fn a_base_with_no_acp_command_needs_acp_args() {
        let h = resolve_in(&acp_profile("claude-code", "acp"), "p").unwrap();
        assert!(h.acp_command().is_err());
        let mut cfg = acp_profile("claude-code", "acp");
        cfg.harness_profiles.get_mut("p").unwrap().acp_args = Some(vec!["serve".into()]);
        assert_eq!(
            resolve_in(&cfg, "p").unwrap().acp_command().unwrap(),
            ["claude", "serve"]
        );
    }

    #[test]
    fn acp_profile_settings_that_cannot_work_are_refused() {
        let with = |edit: fn(&mut HarnessProfileConfig)| {
            let mut cfg = acp_profile("opencode", "acp");
            edit(cfg.harness_profiles.get_mut("p").unwrap());
            resolve_in(&cfg, "p")
        };
        assert!(with(|p| p.acp_args = Some(vec![])).is_err());
        assert!(with(|p| p.args = Some(vec!["--x".into()])).is_err());
        let mut stray = acp_profile("opencode", "pty");
        stray.harness_profiles.get_mut("p").unwrap().acp_args = Some(vec!["acp".into()]);
        assert!(resolve_in(&stray, "p").is_err());
    }

    #[test]
    fn hq_mcp_is_given_only_to_an_acp_profile_that_asks_for_it() {
        let mut cfg = acp_profile("opencode", "acp");
        assert!(!resolve_in(&cfg, "p").unwrap().gives_hq_mcp());
        cfg.harness_profiles.get_mut("p").unwrap().hq_mcp = Some(true);
        assert!(resolve_in(&cfg, "p").unwrap().gives_hq_mcp());
        let mut pty = acp_profile("opencode", "pty");
        pty.harness_profiles.get_mut("p").unwrap().hq_mcp = Some(true);
        assert!(resolve_in(&pty, "p").is_err());
    }

    #[test]
    fn bad_transports_are_refused_when_the_profile_resolves() {
        assert!(resolve_in(&acp_profile("opencode", "carrier-pigeon"), "p").is_err());
        let mut wrapped = acp_profile("opencode", "acp");
        wrapped.harness_profiles.get_mut("p").unwrap().command = Some("wrapper".into());
        assert!(resolve_in(&wrapped, "p").is_err());
    }

    #[test]
    fn all_specs_resolvable_by_name() {
        for s in SPECS {
            assert!(spec_for(s.harness).is_some());
        }
        assert!(spec_for("nonexistent").is_none());
    }

    #[test]
    fn token_patterns_compile() {
        for s in SPECS {
            if let Some(p) = s.token_pattern {
                assert!(
                    regex::Regex::new(p).is_ok(),
                    "bad pattern for {}",
                    s.harness
                );
            }
        }
    }

    #[test]
    fn codex_starts_and_resumes_without_its_update_prompt() {
        let spec = spec_for("codex").unwrap();
        let ResumeStrategy::Args(resume) = spec.resume else {
            panic!("codex resumes by arguments");
        };
        for args in [spec.args, resume] {
            assert!(
                args.windows(2).any(|w| w == ["-c", CODEX_NO_UPDATE_CHECK]),
                "{args:?}"
            );
        }
    }

    #[test]
    fn alias_resolves_to_canonical_spec() {
        let spec = spec_for("agy").expect("agy alias should resolve");
        assert_eq!(spec.harness, "antigravity");
        assert_eq!(canonical_name("agy"), "antigravity");
        assert_eq!(canonical_name("claude-code"), "claude-code");
    }

    fn cfg_with_profile(yaml: &str) -> AgentHostConfig {
        serde_yaml::from_str(&format!("harness_profiles:\n{yaml}")).unwrap()
    }

    #[test]
    fn a_profile_resolves_to_its_base_spec() {
        let cfg = cfg_with_profile("  wrapped:\n    base: claude-code\n    command: w\n");
        let h = resolve_in(&cfg, "wrapped").unwrap();
        assert_eq!(h.name, "wrapped");
        assert_eq!(h.spec.harness, "claude-code");
        assert_eq!(
            h.fresh_args(),
            vec!["--dangerously-skip-permissions".to_string()]
        );
        assert!(known_in(&cfg).contains(&"wrapped".to_string()));
    }

    #[test]
    fn profile_args_replace_the_base_args() {
        let cfg =
            cfg_with_profile("  wrapped:\n    base: claude-code\n    args: [\"--model\", \"x\"]\n");
        let h = resolve_in(&cfg, "wrapped").unwrap();
        assert_eq!(h.fresh_args(), vec!["--model".to_string(), "x".to_string()]);
    }

    #[test]
    fn a_profile_with_an_unknown_base_says_so() {
        let cfg = cfg_with_profile("  bad:\n    base: nope\n");
        let err = resolve_in(&cfg, "bad").unwrap_err().to_string();
        assert!(err.contains("unknown base 'nope'"), "{err}");
    }

    #[test]
    fn builtin_names_resolve_without_profiles_and_unknown_ones_list_the_known() {
        let cfg = AgentHostConfig::default();
        assert_eq!(resolve_in(&cfg, "agy").unwrap().name, "antigravity");
        let err = resolve_in(&cfg, "ghost").unwrap_err().to_string();
        assert!(err.contains("claude-code"), "{err}");
    }

    #[test]
    fn binaries_are_the_executables_the_host_starts() {
        assert_eq!(spec_for("claude-code").unwrap().binary(), "claude");
        assert_eq!(spec_for("agy").unwrap().binary(), "agy");
        assert_eq!(spec_for("cursor").unwrap().binary(), "cursor-agent");
        assert_eq!(spec_for("github-copilot").unwrap().binary(), "copilot");
        let copilot = spec_for("github-copilot").unwrap().args;
        assert_eq!(copilot[0], "--allow-all-tools");
        assert!(copilot.contains(&"--deny-tool=shell(git push)"));
        // Every deny is one argv word: a value split at its space would be read
        // as a second tool name, or as the prompt. And none names a flag, which
        // Copilot ignores (a rule that looks like protection and is not).
        for deny in &copilot[1..] {
            let inner = deny
                .strip_prefix("--deny-tool=shell(")
                .and_then(|d| d.strip_suffix(')'))
                .unwrap_or_else(|| panic!("{deny}"));
            assert!(!inner.split(' ').any(|w| w.starts_with('-')), "{deny}");
        }
    }

    #[test]
    fn general_purpose_agents_are_their_own_category() {
        assert_eq!(
            names_in(HarnessCategory::GeneralPurpose),
            vec!["hermes", "openclaw"]
        );
        assert!(
            spec_for("nanoclaw").is_none(),
            "NanoClaw has no CLI to drive and is not a harness"
        );
        assert!(names_in(HarnessCategory::Coding).contains(&"claude-code"));
        assert_eq!(spec_for("hermes").unwrap().binary(), "hermes");
        assert_eq!(spec_for("hermes").unwrap().args, &["chat"]);
        assert_eq!(spec_for("openclaw").unwrap().args, &["tui", "--local"]);
        assert_eq!(
            resolve_in(&AgentHostConfig::default(), "openclaw")
                .unwrap()
                .category(),
            HarnessCategory::GeneralPurpose
        );
        // A profile takes its base's category.
        let cfg = cfg_with_profile("  hermes-work:\n    base: hermes\n    command: hermes-work\n");
        assert_eq!(
            resolve_in(&cfg, "hermes-work").unwrap().category(),
            HarnessCategory::GeneralPurpose
        );
    }

    #[test]
    fn one_shot_args_substitute_the_brief_and_a_profile_can_replace_them() {
        let hermes = resolve_in(&AgentHostConfig::default(), "hermes").unwrap();
        assert_eq!(
            hermes.one_shot_args("do x").unwrap(),
            vec!["chat", "--oneshot", "-q", "do x"]
        );
        assert_eq!(
            resolve_in(&AgentHostConfig::default(), "openclaw")
                .unwrap()
                .one_shot_args("b")
                .unwrap(),
            vec!["agent", "--local", "--message", "b"]
        );
        assert!(
            resolve_in(&AgentHostConfig::default(), "claude-code")
                .unwrap()
                .one_shot_args("b")
                .is_none()
        );
        let cfg = cfg_with_profile(
            "  my-hermes:\n    base: hermes\n    one_shot_args: [\"run\", \"--once\", \"{brief}\"]\n",
        );
        assert_eq!(
            resolve_in(&cfg, "my-hermes")
                .unwrap()
                .one_shot_args("q")
                .unwrap(),
            vec!["run", "--once", "q"]
        );
    }

    #[test]
    fn cursor_pattern_extracts_chat_id() {
        let re = regex::Regex::new(spec_for("cursor").unwrap().token_pattern.unwrap()).unwrap();
        let caps = re.captures("chat_id: 20260714_142148_ff76c3").unwrap();
        assert_eq!(&caps[1], "20260714_142148_ff76c3");
    }
}
