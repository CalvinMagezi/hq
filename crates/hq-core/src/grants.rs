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

/// Directories and files under HOME no grant may ever open, whatever the profile says.
/// Compared without regard to case, so `.SSH` on a case-insensitive disk is the same place.
pub const DENIED_HOME_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".config/gh",
    ".config/gcloud",
    ".config/cursor",
    ".config/opencode",
    ".config/github-copilot",
    ".local/share/opencode",
    ".local/state/opencode",
    ".password-store",
    "Library/Keychains",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".gitconfig",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".zsh_history",
    ".bash_history",
    ".hq",
    ".claude",
    ".claude.json",
    ".gemini",
    ".codex",
    ".cursor",
    ".copilot",
    ".kimi",
    ".qwen",
    ".pi",
    ".hermes",
    ".openclaw",
    ".agents",
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

/// The home directory profiles are resolved against, empty when the machine has none (so every
/// `~` path then fails the credential check and nothing is opened).
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// `path` with its longest existing ancestor resolved through symlinks, so a link cannot carry a
/// grant outside the directory it names. A path with `..`, or a relative one, is returned as
/// written, for `classify` to refuse.
pub fn canonical_lenient(path: &str) -> String {
    let p = PathBuf::from(path);
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return path.to_string();
    }
    let mut tail = Vec::new();
    let mut at = p.as_path();
    loop {
        if let Ok(real) = at.canonicalize() {
            let mut out = real;
            out.extend(tail.iter().rev());
            return out.to_string_lossy().into_owned();
        }
        let (Some(name), Some(parent)) = (at.file_name(), at.parent()) else {
            return path.to_string();
        };
        tail.push(name.to_os_string());
        at = parent;
    }
}

/// How paths are read on the machine a session runs on, which is not always the machine HQ runs
/// on: the home directory, and a path with its links resolved.
pub trait PathView {
    fn home(&self) -> PathBuf;
    fn canonical(&self, path: &str) -> String;
}

/// The paths of the machine this code runs on.
pub struct LocalPaths;

impl PathView for LocalPaths {
    fn home(&self) -> PathBuf {
        home()
    }
    fn canonical(&self, path: &str) -> String {
        canonical_lenient(path)
    }
}

/// `~` and `~/x` made absolute under `home`; anything else is returned as written.
pub fn expand_home(raw: &str, home: &Path) -> PathBuf {
    match raw.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if raw == "~" => home.to_path_buf(),
        None => PathBuf::from(raw),
    }
}

impl GrantProfile {
    /// The profile with `~` expanded and every path that may never be opened dropped, so a
    /// profile that lists a credential directory by mistake cannot open it.
    pub fn resolved(&self, home: &Path) -> GrantProfile {
        let keep = |list: &[String]| -> Vec<String> {
            list.iter()
                .map(|p| expand_home(p, home))
                .filter(|p| denied_path(p, home).is_none())
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        };
        GrantProfile {
            writable: keep(&self.writable),
            readable: keep(&self.readable),
            loopback_ports: self
                .loopback_ports
                .iter()
                .copied()
                .filter(|p| *p != 0)
                .collect(),
            egress: self.egress.clone(),
        }
    }

    /// What a session that opted into this profile starts with, before it asks for anything.
    pub fn delta(&self, home: &Path) -> SandboxDelta {
        let r = self.resolved(home);
        SandboxDelta {
            writable: r.writable.iter().map(PathBuf::from).collect(),
            readable: r.readable.iter().map(PathBuf::from).collect(),
            loopback_ports: r.loopback_ports,
            allow: r.egress.iter().filter_map(|e| split_host(e)).collect(),
        }
    }
}

/// Names of the starter profiles HQ ships: common toolchains' caches and registries, offered inside
/// the first access request so nobody writes a profile by hand. Paths use `~` for the home of the
/// computer the session runs on. Nothing here is on until the owner allows it for a repository.
pub const STARTER_PROFILES: &[&str] = &["mobile-dev", "node-build", "rust-build"];

/// The contents of a starter profile, or `None` for an unknown name.
///
/// Kept narrow on purpose. A directory the owner's own tools run code from later (cargo's
/// `registry/src`, npm's `_npx`, a pnpm store hard-linked into every project, gradle's `init.d`, the
/// pub `bin`) is never listed, since writing there would plant code that runs outside the sandbox;
/// a helper builds with a cache inside its worktree (`CARGO_HOME`, `npm_config_cache`) instead.
/// Broad hosts that serve anyone's files (`storage.googleapis.com`, `github.com`) and the adb
/// server's port are left out: a helper asks for those one at a time.
pub fn starter_profile(name: &str) -> Option<GrantProfile> {
    let list = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match name {
        "mobile-dev" => Some(GrantProfile {
            writable: list(&[
                "~/.gradle/caches",
                "~/.gradle/wrapper",
                "~/.pub-cache/hosted",
            ]),
            readable: list(&["~/Library/Android/sdk", "~/Android/Sdk"]),
            loopback_ports: Vec::new(),
            egress: list(&[
                "pub.dev",
                "dl.google.com",
                "maven.google.com",
                "repo.maven.apache.org",
                "services.gradle.org",
                "downloads.gradle.org",
                "plugins.gradle.org",
            ]),
        }),
        "node-build" => Some(GrantProfile {
            writable: Vec::new(),
            readable: Vec::new(),
            loopback_ports: Vec::new(),
            egress: list(&["registry.npmjs.org", "registry.yarnpkg.com"]),
        }),
        "rust-build" => Some(GrantProfile {
            writable: Vec::new(),
            readable: Vec::new(),
            loopback_ports: Vec::new(),
            egress: list(&["crates.io", "index.crates.io", "static.crates.io"]),
        }),
        _ => None,
    }
}

/// A standing rule's resource as a request would name it: `~` made absolute under `home`.
pub fn expand_resource(capability: Capability, resource: &str, home: &Path) -> String {
    match capability {
        Capability::FsWrite | Capability::FsRead => {
            expand_home(resource, home).to_string_lossy().into_owned()
        }
        _ => resource.to_string(),
    }
}

/// Whether a standing rule (`rule`, already expanded) covers `req`: a directory covers what is
/// under it, a port or host only itself. The caller has already checked that `req` is not refused.
pub fn rule_covers(rule_capability: Capability, rule_resource: &str, req: &GrantRequest) -> bool {
    match (rule_capability, req.capability) {
        (Capability::FsWrite, Capability::FsWrite | Capability::FsRead)
        | (Capability::FsRead, Capability::FsRead) => {
            Path::new(&req.resource).starts_with(rule_resource)
        }
        (a, b) if a == b => rule_resource == req.resource,
        _ => false,
    }
}

/// Which starter profile `req` falls inside, if any, so the first card can offer "allow
/// <starter> for this repository". `home` is the session computer's.
pub fn starter_for(req: &GrantRequest, home: &Path) -> Option<&'static str> {
    STARTER_PROFILES.iter().copied().find(|name| {
        let Some(p) = starter_profile(name) else {
            return false;
        };
        let covers = |cap: Capability, items: &[String]| {
            items.iter().any(|raw| {
                let r = normalize(&GrantRequest {
                    capability: cap,
                    resource: expand_resource(cap, raw, home),
                });
                rule_covers(cap, &r.resource, req)
            })
        };
        covers(Capability::FsWrite, &p.writable)
            || covers(Capability::FsRead, &p.readable)
            || covers(
                Capability::Loopback,
                &p.loopback_ports
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>(),
            )
            || covers(Capability::Egress, &p.egress)
    })
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

fn lowered(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Why `path` may never be inside a writable or readable root, if so.
pub fn denied_path(path: &Path, home: &Path) -> Option<RefuseReason> {
    if !path.is_absolute() {
        return Some(RefuseReason::NotAbsolute);
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Some(RefuseReason::Traversal);
    }
    let (p, h) = (lowered(path), lowered(home));
    if h.starts_with(&p) {
        return Some(RefuseReason::HomeRoot);
    }
    let rest = p.strip_prefix(h.as_slice())?;
    let named = DENIED_HOME_DIRS.iter().any(|d| {
        let d: Vec<String> = Path::new(d)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect();
        rest.starts_with(&d) || d.starts_with(rest)
    });
    let other_claude = rest
        .first()
        .is_some_and(|f| f.starts_with(OTHER_CLAUDE_PREFIX));
    (named || other_claude).then_some(RefuseReason::Credentials)
}

/// The request in its one canonical spelling, so asking twice in different words is asking once.
/// What cannot be read stays as written for `classify` to refuse.
pub fn normalize(req: &GrantRequest) -> GrantRequest {
    let resource = match req.capability {
        Capability::Loopback => req
            .resource
            .trim()
            .parse::<u16>()
            .map(|p| p.to_string())
            .unwrap_or_else(|_| req.resource.clone()),
        Capability::Egress => split_host(&req.resource)
            .map(|(h, p)| format!("{h}:{p}"))
            .unwrap_or_else(|| req.resource.clone()),
        _ => req.resource.clone(),
    };
    GrantRequest {
        capability: req.capability,
        resource,
    }
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
            home: "/home/user".into(),
            worktree: "/home/user/wt/app".into(),
            profile,
        }
    }

    fn mobile() -> GrantProfile {
        GrantProfile {
            writable: vec!["/home/user/.gradle".into()],
            readable: vec!["/home/user/Android/sdk".into()],
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
            "/home/user/.gradle/caches",
            Some(mobile()),
        );
        let Decision::Approve(delta) = d else {
            panic!("{d:?}")
        };
        assert_eq!(
            delta.writable,
            vec![PathBuf::from("/home/user/.gradle/caches")]
        );
        assert!(delta.readable.is_empty() && delta.allow.is_empty());
    }

    #[test]
    fn reading_is_allowed_under_a_writable_root_but_writing_is_not_allowed_under_a_readable_one() {
        let p = Some(mobile());
        assert!(matches!(
            run(Capability::FsRead, "/home/user/.gradle", p.clone()),
            Decision::Approve(_)
        ));
        assert_eq!(
            run(Capability::FsWrite, "/home/user/Android/sdk/x", p),
            Decision::Ask(AskReason::OutsideProfile)
        );
    }

    #[test]
    fn the_task_worktree_is_approved_and_a_sibling_is_not() {
        let p = Some(mobile());
        assert!(matches!(
            run(Capability::FsWrite, "/home/user/wt/app/build", p.clone()),
            Decision::Approve(_)
        ));
        assert_eq!(
            run(Capability::FsWrite, "/home/user/wt/other", p),
            Decision::Ask(AskReason::OutsideProfile)
        );
    }

    #[test]
    fn no_profile_asks_and_never_approves() {
        assert_eq!(
            run(Capability::FsWrite, "/home/user/wt/app/x", None),
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
        p.writable.push("/home/user/.ssh".into());
        for path in [
            "/home/user/.ssh",
            "/home/user/.ssh/id_ed25519",
            "/home/user/.config/gh/hosts.yml",
            "/home/user/.hq/config.yaml",
            "/home/user/.claude-work/x",
            "/home/user/.aws",
        ] {
            assert_eq!(
                run(Capability::FsRead, path, Some(p.clone())),
                Decision::Refuse(RefuseReason::Credentials),
                "{path}"
            );
        }
        // Without a profile the refusal still wins over the ask.
        assert_eq!(
            run(Capability::FsWrite, "/home/user/.ssh", None),
            Decision::Refuse(RefuseReason::Credentials)
        );
    }

    #[test]
    fn home_its_parents_relative_paths_and_dotdot_are_refused() {
        let p = Some(mobile());
        for (path, why) in [
            ("/home/user", RefuseReason::HomeRoot),
            ("/home", RefuseReason::HomeRoot),
            ("/", RefuseReason::HomeRoot),
            ("gradle", RefuseReason::NotAbsolute),
            ("/home/user/wt/app/../../.ssh", RefuseReason::Traversal),
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
            run(Capability::FsRead, "/home/user/.config", Some(mobile())),
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
    fn a_symlink_cannot_carry_a_path_outside_the_directory_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let got = canonical_lenient(&format!("{}/link/new/dir", root.display()));
        assert_eq!(got, format!("{}/real/new/dir", root.display()));
        assert_eq!(
            canonical_lenient("/a/../b"),
            "/a/../b",
            "left for classify to refuse"
        );
        assert_eq!(canonical_lenient("rel"), "rel");
    }

    #[test]
    fn a_profile_expands_home_and_drops_credential_paths() {
        let p = GrantProfile {
            writable: vec!["~/.gradle".into(), "~/.ssh".into()],
            readable: vec!["/home/user".into()],
            loopback_ports: vec![0, 5037],
            egress: vec!["pub.dev".into(), "*.bad".into()],
        };
        let d = p.delta(Path::new("/home/user"));
        assert_eq!(d.writable, vec![PathBuf::from("/home/user/.gradle")]);
        assert!(d.readable.is_empty(), "home itself is dropped");
        assert_eq!(d.loopback_ports, vec![5037]);
        assert_eq!(d.allow, vec![("pub.dev".to_string(), 443)]);
    }

    #[test]
    fn credential_paths_match_without_regard_to_case_and_more_stores_are_listed() {
        let p = Some(mobile());
        for path in [
            "/home/user/.SSH/id_ed25519",
            "/HOME/USER/.Aws",
            "/home/user/Library/Keychains/login.keychain-db",
            "/home/user/.config/gcloud/x",
            "/home/user/.gitconfig",
            "/home/user/.hermes/memory",
        ] {
            assert!(
                matches!(
                    run(Capability::FsRead, path, p.clone()),
                    Decision::Refuse(_)
                ),
                "{path}"
            );
        }
        assert_eq!(
            run(Capability::FsRead, "/Home/User", p),
            Decision::Refuse(RefuseReason::HomeRoot)
        );
    }

    #[test]
    fn spellings_of_one_request_normalize_to_one() {
        let n = |c, r: &str| normalize(&req(c, r)).resource;
        assert_eq!(n(Capability::Loopback, " 05037 "), "5037");
        assert_eq!(n(Capability::Egress, "Pub.Dev"), "pub.dev:443");
        assert_eq!(n(Capability::Egress, "pub.dev:443"), "pub.dev:443");
        assert_eq!(
            n(Capability::Egress, "*.bad"),
            "*.bad",
            "left for classify to refuse"
        );
    }

    #[test]
    fn starters_cover_their_caches_and_registries_and_nothing_else() {
        let home = Path::new("/home/user");
        let r = |c, x: &str| GrantRequest {
            capability: c,
            resource: x.into(),
        };
        assert_eq!(
            starter_for(&r(Capability::FsWrite, "/home/user/.gradle/caches"), home),
            Some("mobile-dev")
        );
        assert_eq!(
            starter_for(&r(Capability::Egress, "registry.npmjs.org:443"), home),
            Some("node-build")
        );
        assert_eq!(
            starter_for(&r(Capability::Egress, "index.crates.io:443"), home),
            Some("rust-build")
        );
        // Places the owner's own tools run code from, the adb port and broad hosts are in no starter.
        for risky in [
            "/home/user/.cargo/registry/src",
            "/home/user/.npm/_npx",
            "/home/user/.gradle/init.d",
            "/home/user/.pub-cache/bin",
        ] {
            assert_eq!(
                starter_for(&r(Capability::FsWrite, risky), home),
                None,
                "{risky}"
            );
        }
        assert_eq!(starter_for(&r(Capability::Loopback, "5037"), home), None);
        assert_eq!(
            starter_for(&r(Capability::Egress, "github.com:443"), home),
            None
        );
        assert_eq!(
            starter_for(&r(Capability::FsWrite, "/home/user/projects"), home),
            None
        );
        assert_eq!(
            starter_for(&r(Capability::Egress, "evil.example.org:443"), home),
            None
        );
        for name in STARTER_PROFILES {
            let p = starter_profile(name).unwrap();
            assert_eq!(
                p.resolved(home).writable.len(),
                p.writable.len(),
                "{name} lists no credential path"
            );
        }
        assert!(starter_profile("nope").is_none());
    }

    #[test]
    fn a_rule_covers_what_is_under_it_and_a_write_rule_covers_reads() {
        let r = |c, x: &str| GrantRequest {
            capability: c,
            resource: x.into(),
        };
        assert!(rule_covers(
            Capability::FsWrite,
            "/a/b",
            &r(Capability::FsWrite, "/a/b/c")
        ));
        assert!(rule_covers(
            Capability::FsWrite,
            "/a/b",
            &r(Capability::FsRead, "/a/b")
        ));
        assert!(!rule_covers(
            Capability::FsRead,
            "/a/b",
            &r(Capability::FsWrite, "/a/b")
        ));
        assert!(!rule_covers(
            Capability::FsWrite,
            "/a/b",
            &r(Capability::FsWrite, "/a/bc")
        ));
        assert!(rule_covers(
            Capability::Egress,
            "pub.dev:443",
            &r(Capability::Egress, "pub.dev:443")
        ));
        assert!(!rule_covers(
            Capability::Egress,
            "pub.dev:443",
            &r(Capability::Loopback, "443")
        ));
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
