//! Delegated sandbox grants: what a helper may ask HQ for, and the pure decision on each request.
//!
//! `classify` takes a typed request, the task's context and the owner's profile, and returns
//! approve, ask or refuse. No model text reaches it: the helper's reason is stored elsewhere and
//! shown to the owner as the helper's words. Paths must already be canonical (the caller resolves
//! symlinks); this module refuses anything relative or with `..` so it never needs the disk.
//! Plan: `docs/plans/delegated-sandbox-grants.md`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Directories under HOME no grant may ever open, whatever the profile says.
pub const DENIED_HOME_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".docker",
    ".config/gh",
    ".netrc",
    ".npmrc",
    ".git-credentials",
    ".hq",
    ".claude",
    ".claude.json",
    ".gemini",
    ".codex",
    ".cursor",
    ".copilot",
];
/// Another Claude account's directory (`~/.claude-<name>`).
const OTHER_CLAUDE_PREFIX: &str = ".claude-";
const DEFAULT_EGRESS_PORT: u16 = 443;

/// What a request is for. Recipes and devices are brokered rather than widened, so they are not
/// here yet: a request for them is `Unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    FsWrite,
    FsRead,
    Loopback,
    Egress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRequest {
    pub capability: Capability,
    /// A path, a port, or `host[:port]`, by capability.
    pub resource: String,
}

/// The owner's standing policy for one kind of work. Empty means nothing is approved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantProfile {
    #[serde(default)]
    pub writable: Vec<String>,
    #[serde(default)]
    pub readable: Vec<String>,
    #[serde(default)]
    pub loopback_ports: Vec<u16>,
    /// `host` or `host:port`, exact names only.
    #[serde(default)]
    pub egress: Vec<String>,
}

/// `agent_host.grants`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrantsConfig {
    /// Named profiles a task or spawn opts into. Nothing is enabled until the owner names one.
    #[serde(default)]
    pub profiles: BTreeMap<String, GrantProfile>,
}

/// What `classify` knows about the session asking.
#[derive(Debug, Clone)]
pub struct TaskContext {
    pub home: PathBuf,
    pub worktree: PathBuf,
    /// The profile the task opted into, with `~` already expanded; `None` when it opted into none.
    pub profile: Option<GrantProfile>,
}

/// An additive change to one session's sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxDelta {
    #[serde(default)]
    pub writable: Vec<PathBuf>,
    #[serde(default)]
    pub readable: Vec<PathBuf>,
    #[serde(default)]
    pub loopback_ports: Vec<u16>,
    /// `(host, port)` the egress proxy may reach.
    #[serde(default)]
    pub allow: Vec<(String, u16)>,
}

impl SandboxDelta {
    pub fn is_empty(&self) -> bool {
        self.writable.is_empty()
            && self.readable.is_empty()
            && self.loopback_ports.is_empty()
            && self.allow.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskReason {
    NoProfile,
    OutsideProfile,
    Unsupported,
}

impl AskReason {
    pub fn text(self) -> &'static str {
        match self {
            Self::NoProfile => "this task has no access profile",
            Self::OutsideProfile => "the request is outside the task's access profile",
            Self::Unsupported => "this kind of access is not one HQ can apply by itself",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefuseReason {
    Invalid,
    NotAbsolute,
    Traversal,
    Credentials,
    HomeRoot,
    PortZero,
}

impl RefuseReason {
    pub fn text(self) -> &'static str {
        match self {
            Self::Invalid => "the request is malformed",
            Self::NotAbsolute => "paths must be absolute",
            Self::Traversal => "paths may not contain `..`",
            Self::Credentials => {
                "credentials, HQ's own files and other agents' state are never opened"
            }
            Self::HomeRoot => "the home directory and its parents are never opened",
            Self::PortZero => "port 0 is not a port",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Approve(SandboxDelta),
    Ask(AskReason),
    Refuse(RefuseReason),
}

/// Why `path` may never be inside a writable or readable root, if so.
pub fn denied_path(path: &Path, home: &Path) -> Option<RefuseReason> {
    if !path.is_absolute() {
        return Some(RefuseReason::NotAbsolute);
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Some(RefuseReason::Traversal);
    }
    if home.starts_with(path) {
        return Some(RefuseReason::HomeRoot);
    }
    let in_home = path.strip_prefix(home).ok()?;
    let first = in_home.components().next()?.as_os_str().to_string_lossy();
    let named = DENIED_HOME_DIRS
        .iter()
        .any(|d| in_home.starts_with(d) || Path::new(d).starts_with(in_home));
    (named || first.starts_with(OTHER_CLAUDE_PREFIX)).then_some(RefuseReason::Credentials)
}

fn under_any(path: &Path, roots: &[String]) -> bool {
    roots.iter().any(|r| path.starts_with(r))
}

fn classify_path(req: &GrantRequest, ctx: &TaskContext, profile: &GrantProfile) -> Decision {
    let path = PathBuf::from(&req.resource);
    if let Some(why) = denied_path(&path, &ctx.home) {
        return Decision::Refuse(why);
    }
    let write = req.capability == Capability::FsWrite;
    let own = path.starts_with(&ctx.worktree);
    let listed = if write {
        under_any(&path, &profile.writable)
    } else {
        under_any(&path, &profile.readable) || under_any(&path, &profile.writable)
    };
    if !(own || listed) {
        return Decision::Ask(AskReason::OutsideProfile);
    }
    let delta = if write {
        SandboxDelta {
            writable: vec![path],
            ..Default::default()
        }
    } else {
        SandboxDelta {
            readable: vec![path],
            ..Default::default()
        }
    };
    Decision::Approve(delta)
}

fn classify_loopback(req: &GrantRequest, profile: &GrantProfile) -> Decision {
    let Ok(port) = req.resource.trim().parse::<u16>() else {
        return Decision::Refuse(RefuseReason::Invalid);
    };
    if port == 0 {
        return Decision::Refuse(RefuseReason::PortZero);
    }
    if !profile.loopback_ports.contains(&port) {
        return Decision::Ask(AskReason::OutsideProfile);
    }
    Decision::Approve(SandboxDelta {
        loopback_ports: vec![port],
        ..Default::default()
    })
}

/// `host[:port]` with no wildcard, no scheme and no path.
fn split_host(entry: &str) -> Option<(String, u16)> {
    let entry = entry.trim().to_ascii_lowercase();
    let (host, port) = match entry.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().ok()?),
        None => (entry, DEFAULT_EGRESS_PORT),
    };
    let clean = !host.is_empty()
        && port != 0
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    clean.then_some((host, port))
}

fn classify_egress(req: &GrantRequest, profile: &GrantProfile) -> Decision {
    let Some((host, port)) = split_host(&req.resource) else {
        return Decision::Refuse(RefuseReason::Invalid);
    };
    let listed = profile
        .egress
        .iter()
        .filter_map(|e| split_host(e))
        .any(|(h, p)| h == host && p == port);
    if !listed {
        return Decision::Ask(AskReason::OutsideProfile);
    }
    Decision::Approve(SandboxDelta {
        allow: vec![(host, port)],
        ..Default::default()
    })
}

pub fn classify(req: &GrantRequest, ctx: &TaskContext) -> Decision {
    if req.resource.is_empty() || req.resource.contains('\0') {
        return Decision::Refuse(RefuseReason::Invalid);
    }
    // Refusals do not depend on a profile: a credential path is refused with or without one.
    if matches!(req.capability, Capability::FsWrite | Capability::FsRead)
        && let Some(why) = denied_path(Path::new(&req.resource), &ctx.home)
    {
        return Decision::Refuse(why);
    }
    let Some(profile) = &ctx.profile else {
        return Decision::Ask(AskReason::NoProfile);
    };
    match req.capability {
        Capability::FsWrite | Capability::FsRead => classify_path(req, ctx, profile),
        Capability::Loopback => classify_loopback(req, profile),
        Capability::Egress => classify_egress(req, profile),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(profile: Option<GrantProfile>) -> TaskContext {
        TaskContext {
            home: "/home/o".into(),
            worktree: "/home/o/wt/app".into(),
            profile,
        }
    }

    fn mobile() -> GrantProfile {
        GrantProfile {
            writable: vec!["/home/o/.gradle".into()],
            readable: vec!["/home/o/Android/sdk".into()],
            loopback_ports: vec![5037],
            egress: vec!["pub.dev".into(), "repo.example.org:8443".into()],
        }
    }

    fn req(capability: Capability, resource: &str) -> GrantRequest {
        GrantRequest {
            capability,
            resource: resource.into(),
        }
    }

    fn run(capability: Capability, resource: &str, profile: Option<GrantProfile>) -> Decision {
        classify(&req(capability, resource), &ctx(profile))
    }

    #[test]
    fn a_listed_cache_is_approved_with_exactly_that_path() {
        let d = run(
            Capability::FsWrite,
            "/home/o/.gradle/caches",
            Some(mobile()),
        );
        let Decision::Approve(delta) = d else {
            panic!("{d:?}")
        };
        assert_eq!(
            delta.writable,
            vec![PathBuf::from("/home/o/.gradle/caches")]
        );
        assert!(delta.readable.is_empty() && delta.allow.is_empty());
    }

    #[test]
    fn reading_is_allowed_under_a_writable_root_but_writing_is_not_allowed_under_a_readable_one() {
        let p = Some(mobile());
        assert!(matches!(
            run(Capability::FsRead, "/home/o/.gradle", p.clone()),
            Decision::Approve(_)
        ));
        assert_eq!(
            run(Capability::FsWrite, "/home/o/Android/sdk/x", p),
            Decision::Ask(AskReason::OutsideProfile)
        );
    }

    #[test]
    fn the_task_worktree_is_approved_and_a_sibling_is_not() {
        let p = Some(mobile());
        assert!(matches!(
            run(Capability::FsWrite, "/home/o/wt/app/build", p.clone()),
            Decision::Approve(_)
        ));
        assert_eq!(
            run(Capability::FsWrite, "/home/o/wt/other", p),
            Decision::Ask(AskReason::OutsideProfile)
        );
    }

    #[test]
    fn no_profile_asks_and_never_approves() {
        assert_eq!(
            run(Capability::FsWrite, "/home/o/wt/app/x", None),
            Decision::Ask(AskReason::NoProfile)
        );
        assert_eq!(
            run(Capability::Loopback, "5037", None),
            Decision::Ask(AskReason::NoProfile)
        );
    }

    #[test]
    fn credentials_and_hq_are_refused_even_when_a_profile_lists_them() {
        let mut p = mobile();
        p.writable.push("/home/o/.ssh".into());
        for path in [
            "/home/o/.ssh",
            "/home/o/.ssh/id_ed25519",
            "/home/o/.config/gh/hosts.yml",
            "/home/o/.hq/config.yaml",
            "/home/o/.claude-work/x",
            "/home/o/.aws",
        ] {
            assert_eq!(
                run(Capability::FsRead, path, Some(p.clone())),
                Decision::Refuse(RefuseReason::Credentials),
                "{path}"
            );
        }
        // Without a profile the refusal still wins over the ask.
        assert_eq!(
            run(Capability::FsWrite, "/home/o/.ssh", None),
            Decision::Refuse(RefuseReason::Credentials)
        );
    }

    #[test]
    fn home_its_parents_relative_paths_and_dotdot_are_refused() {
        let p = Some(mobile());
        for (path, why) in [
            ("/home/o", RefuseReason::HomeRoot),
            ("/home", RefuseReason::HomeRoot),
            ("/", RefuseReason::HomeRoot),
            ("gradle", RefuseReason::NotAbsolute),
            ("/home/o/wt/app/../../.ssh", RefuseReason::Traversal),
        ] {
            assert_eq!(
                run(Capability::FsWrite, path, p.clone()),
                Decision::Refuse(why),
                "{path}"
            );
        }
    }

    #[test]
    fn a_parent_of_a_credential_directory_is_refused_so_a_root_cannot_contain_it() {
        assert_eq!(
            run(Capability::FsRead, "/home/o/.config", Some(mobile())),
            Decision::Refuse(RefuseReason::Credentials)
        );
    }

    #[test]
    fn loopback_needs_a_listed_port_and_port_zero_is_refused() {
        let p = Some(mobile());
        assert!(matches!(
            run(Capability::Loopback, "5037", p.clone()),
            Decision::Approve(d) if d.loopback_ports == vec![5037]
        ));
        assert_eq!(
            run(Capability::Loopback, "5038", p.clone()),
            Decision::Ask(AskReason::OutsideProfile)
        );
        assert_eq!(
            run(Capability::Loopback, "0", p.clone()),
            Decision::Refuse(RefuseReason::PortZero)
        );
        assert_eq!(
            run(Capability::Loopback, "any", p),
            Decision::Refuse(RefuseReason::Invalid)
        );
    }

    #[test]
    fn egress_matches_exact_hosts_and_ports_only() {
        let p = Some(mobile());
        assert!(matches!(
            run(Capability::Egress, "PUB.dev", p.clone()),
            Decision::Approve(d) if d.allow == vec![("pub.dev".to_string(), 443)]
        ));
        assert!(matches!(
            run(Capability::Egress, "repo.example.org:8443", p.clone()),
            Decision::Approve(_)
        ));
        for ask in ["repo.example.org", "evil.pub.dev", "pub.dev:80"] {
            assert_eq!(
                run(Capability::Egress, ask, p.clone()),
                Decision::Ask(AskReason::OutsideProfile),
                "{ask}"
            );
        }
        for bad in ["*.pub.dev", "https://pub.dev", "pub.dev/x", "a b"] {
            assert_eq!(
                run(Capability::Egress, bad, p.clone()),
                Decision::Refuse(RefuseReason::Invalid),
                "{bad}"
            );
        }
    }

    #[test]
    fn empty_and_nul_resources_are_refused() {
        assert_eq!(
            run(Capability::FsRead, "", Some(mobile())),
            Decision::Refuse(RefuseReason::Invalid)
        );
        assert_eq!(
            run(Capability::FsRead, "/a\0b", Some(mobile())),
            Decision::Refuse(RefuseReason::Invalid)
        );
    }
}
