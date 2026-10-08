//! Per-harness session descriptors: how to spawn each CLI interactively, how
//! to resume a prior session, and how to harvest a resume token from output.
//! Configured profiles (`agent_host.harness_profiles`) resolve here too, so every
//! caller that accepts a harness name accepts a profile name.

use anyhow::{Result, bail};
use hq_core::config::{HarnessProfileConfig, AgentHostConfig, HqConfig};

const PERMISSION_MODE_FLAG: &str = "--permission-mode";
const PERMISSION_MODE_MANUAL: &str = "manual";

/// Flags that make an agent run commands without asking. HQ refuses to launch with any of them,
/// in a built-in spec or a configured profile, so approval dialogs always reach a person or the
/// Drive loop.
const BYPASS_FLAGS: &[&str] = &[
    "--dangerously-skip-permissions",
    "--allow-dangerously-skip-permissions",
    "--dangerously-bypass-approvals-and-sandbox",
    "--dangerously-bypass-hook-trust",
    "--yolo",
    "--force",
    "--full-auto",
    "--approve-mcps",
];

/// The first bypass flag in `text`, matched on whole words.
pub fn find_bypass_flag(text: &str) -> Option<&'static str> {
    text.split_whitespace()
        .find_map(|word| BYPASS_FLAGS.iter().copied().find(|flag| *flag == word))
}

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

#[derive(Debug, Clone)]
pub struct HarnessSessionSpec {
    pub harness: &'static str,
    /// host agent kind (`agent start --kind`). The host resolves the CLI on
    /// the target host, so HQ never needs the binary on its own machine.
    pub kind: &'static str,
    /// Args for a fresh interactive spawn.
    pub args: &'static [&'static str],
    pub resume: ResumeStrategy,
    /// Regex with one capture group extracting a resume token from session output.
    pub token_pattern: Option<&'static str>,
    /// Substring of a workspace-trust dialog whose default answer is "trust".
    /// Only for harnesses verified to default that way: when spawn finds the
    /// agent blocked on it, spawn presses Enter. Any other blocked dialog is
    /// left for the caller to read and answer, since some CLIs now default a
    /// trust prompt to "No, exit".
    pub trust_pattern: Option<&'static str>,
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

/// The known interactive-session harnesses.
///
/// Resume flags verified against each CLI where known; harnesses marked
/// `ResumeStrategy::None` need their flag confirmed before wiring
/// (tracked as adapter follow-ups).
pub const SPECS: &[HarnessSessionSpec] = &[
    HarnessSessionSpec {
        harness: "claude-code",
        kind: "claude",
        // Explicit so a bypass default in the user's own settings cannot apply. Approval dialogs
        // are answered by the person or the Drive loop, never skipped.
        args: &[PERMISSION_MODE_FLAG, PERMISSION_MODE_MANUAL],
        // `claude --resume <id>` is exact; `claude -c` continues the most recent
        // session in the cwd, used until the id is known.
        resume: ResumeStrategy::TokenOrArgs {
            with_token: &[PERMISSION_MODE_FLAG, PERMISSION_MODE_MANUAL, "--resume", "{token}"],
            otherwise: &[PERMISSION_MODE_FLAG, PERMISSION_MODE_MANUAL, "-c"],
        },
        token_pattern: None,
        trust_pattern: None,
    },
    HarnessSessionSpec {
        harness: "qwen",
        kind: "qwen",
        args: &["--auth-type", "qwen-oauth"],
        resume: ResumeStrategy::Args(&["--auth-type", "qwen-oauth", "--continue"]),
        token_pattern: None,
        trust_pattern: None,
    },
    HarnessSessionSpec {
        harness: "cursor",
        kind: "cursor",
        // `--force` (run everything) is deliberately absent. `--trust` only works in headless mode;
        // the interactive trust dialog is answered at launch.
        args: &[],
        resume: ResumeStrategy::Args(&["--resume", "{token}"]),
        token_pattern: Some(r"chat[_-]id[:=]\s*([A-Za-z0-9_-]+)"),
        trust_pattern: None,
    },
    HarnessSessionSpec {
        harness: "pi",
        kind: "pi",
        args: &[],
        resume: ResumeStrategy::SessionDir,
        token_pattern: None,
        trust_pattern: None,
    },
    HarnessSessionSpec {
        harness: "codex",
        kind: "codex",
        // The background daemon check reads other processes, which the sandbox forbids.
        args: &["--no-daemon"],
        resume: ResumeStrategy::Args(&["--no-daemon", "resume", "--last"]),
        token_pattern: None,
        trust_pattern: None,
    },
    HarnessSessionSpec {
        harness: "opencode",
        kind: "opencode",
        args: &[],
        resume: ResumeStrategy::Args(&["--continue"]),
        token_pattern: None,
        trust_pattern: None,
    },
    HarnessSessionSpec {
        harness: "kimi",
        kind: "kimi",
        args: &[],
        resume: ResumeStrategy::None,
        token_pattern: None,
        trust_pattern: None,
    },
    HarnessSessionSpec {
        harness: "antigravity",
        kind: "agy",
        args: &[],
        // `agy -c` continues the most recent conversation in the cwd.
        resume: ResumeStrategy::Args(&["-c"]),
        token_pattern: None,
        // agy asks "Do you trust the contents of this project?" on first use
        // of a cwd; default selection is "Yes, I trust this folder".
        trust_pattern: Some("trust the contents"),
    },
    HarnessSessionSpec {
        harness: "github-copilot",
        kind: "copilot",
        args: &[],
        resume: ResumeStrategy::None,
        token_pattern: None,
        trust_pattern: None,
    },
];

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
}

impl Harness {
    /// Arguments for a fresh spawn: the profile's when it sets any.
    pub fn fresh_args(&self) -> Vec<String> {
        match self.profile.as_ref().and_then(|p| p.args.as_ref()) {
            Some(args) => args.clone(),
            None => self.spec.args.iter().map(|a| a.to_string()).collect(),
        }
    }
}

/// Resolve `name` against the profiles in `cfg`, then the built-in harnesses.
/// A profile may share a built-in's name to replace how it launches.
pub fn resolve_in(cfg: &AgentHostConfig, name: &str) -> Result<Harness> {
    if let Some(profile) = cfg.harness_profiles.get(name) {
        let Some(spec) = spec_for(&profile.base) else {
            bail!(
                "harness profile '{name}' names unknown base '{}' (known: {})",
                profile.base,
                builtin_names().join(", ")
            );
        };
        let offered = profile.args.iter().flatten().map(String::as_str).chain(profile.command.as_deref());
        if let Some(flag) = offered.into_iter().find_map(find_bypass_flag) {
            bail!("harness profile '{name}' uses '{flag}', which skips approval prompts; HQ does not launch agents in a bypass mode");
        }
        return Ok(Harness {
            name: name.to_string(),
            spec,
            profile: Some(profile.clone()),
        });
    }
    match spec_for(name) {
        Some(spec) => Ok(Harness {
            name: spec.harness.to_string(),
            spec,
            profile: None,
        }),
        None => bail!(
            "unknown session harness '{name}' (known: {})",
            known_in(cfg).join(", ")
        ),
    }
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

fn agent_host_config() -> AgentHostConfig {
    HqConfig::load().map(|c| c.agent_host).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(h.fresh_args(), vec!["--permission-mode".to_string(), "manual".to_string()]);
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
    fn no_builtin_spec_launches_in_a_bypass_mode() {
        for s in SPECS {
            let resume: Vec<&[&str]> = match s.resume {
                ResumeStrategy::Args(a) => vec![a],
                ResumeStrategy::TokenOrArgs { with_token, otherwise } => vec![with_token, otherwise],
                _ => vec![],
            };
            for arg in s.args.iter().chain(resume.into_iter().flatten()) {
                assert!(find_bypass_flag(arg).is_none(), "{} uses {arg}", s.harness);
            }
        }
    }

    #[test]
    fn a_profile_cannot_reintroduce_a_bypass_flag() {
        for yaml in [
            "  p:\n    base: claude-code\n    args: [\"--dangerously-skip-permissions\"]\n",
            "  p:\n    base: cursor\n    command: \"cursor-agent --force\"\n",
        ] {
            let err = resolve_in(&cfg_with_profile(yaml), "p").unwrap_err().to_string();
            assert!(err.contains("bypass mode"), "{err}");
        }
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
    }

    #[test]
    fn cursor_pattern_extracts_chat_id() {
        let re = regex::Regex::new(spec_for("cursor").unwrap().token_pattern.unwrap()).unwrap();
        let caps = re.captures("chat_id: 20260714_142148_ff76c3").unwrap();
        assert_eq!(&caps[1], "20260714_142148_ff76c3");
    }
}
