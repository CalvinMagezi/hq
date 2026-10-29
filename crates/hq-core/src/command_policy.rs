//! Command policies: which commands a coding agent may run without asking the owner
//! (docs/plans/remote-driven-teams.md, phase 2). Policy is code: a model never approves a
//! command, it only sees what a policy escalates.
//!
//! A policy names commands to allow and to deny, and what happens to everything else
//! (`otherwise`). A command line is split into its commands at `&&`, `||`, `;`, `|` and `&`;
//! it is allowed only when every one of them is, and a deny anywhere wins. A pattern names
//! the leading words of a command (`git push`) and, optionally, flags that may appear
//! anywhere after them (`git push --force` also matches `git push origin main --force`; a
//! short flag matches inside a cluster, so `rm -r` matches `rm -rf`). Command substitution,
//! `eval` and inline interpreter code (`sh -c`, `python3 -c`, a heredoc) escalate unless the
//! policy allows scripts. Denies guard against a slip; what stops a push for certain is the
//! agent not holding the credential.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// The built-in policy for a session whose purpose is `code`.
pub const BUILDER: &str = "builder";
/// The built-in policy for a session whose purpose is `review`.
pub const REVIEWER: &str = "reviewer";
/// The name that asks for no policy: every dialog goes to the owner, as before policies.
pub const NONE: &str = "none";

/// Longest `extends` chain followed.
const MAX_EXTENDS: usize = 8;
/// Longest command line judged; a longer one escalates.
const MAX_COMMAND: usize = 16 * 1024;

/// What happens to a command no pattern names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Otherwise {
    Deny,
    Escalate,
    Allow,
}

/// Whether inline scripts (`sh -c`, `python3 -c`, `$(...)`, a heredoc) are judged like any
/// other command or always go to the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scripts {
    Escalate,
    Allow,
}

/// One policy as the owner writes it under `agent_host.command_policies`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Another policy whose lists this one starts from.
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    /// Inherited allow entries to drop.
    #[serde(default)]
    pub allow_remove: Vec<String>,
    /// Inherited deny entries to drop.
    #[serde(default)]
    pub deny_remove: Vec<String>,
    /// Inherited when unset; `escalate` at the root.
    #[serde(default)]
    pub otherwise: Option<Otherwise>,
    /// Inherited when unset; `escalate` at the root.
    #[serde(default)]
    pub scripts: Option<Scripts>,
}

/// A policy with its inheritance applied: what the launch hooks and the supervisor judge with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub otherwise: Otherwise,
    pub scripts: Scripts,
}

/// What a policy says about one command, path or edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow { rule: String },
    Deny { rule: String },
    Escalate { reason: String },
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Allow { .. } => "allow",
            Decision::Deny { .. } => "deny",
            Decision::Escalate { .. } => "escalate",
        }
    }

    /// The rule or reason behind it.
    pub fn why(&self) -> &str {
        match self {
            Decision::Allow { rule } | Decision::Deny { rule } => rule,
            Decision::Escalate { reason } => reason,
        }
    }
}

fn list(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// The built-in policies, generic enough for any repository. The owner's own entries of the
/// same name replace them; project commands (a framework's codegen, a deploy) belong there.
pub fn builtins() -> BTreeMap<String, PolicyConfig> {
    let builder = PolicyConfig {
        allow: list(&[
            // Reading the tree.
            "ls",
            "cat",
            "head",
            "tail",
            "wc",
            "grep",
            "rg",
            "pwd",
            "which",
            "file",
            "stat",
            "du",
            "tree",
            "diff",
            "sort",
            "uniq",
            "echo",
            // Git that stays in the session's own tree.
            "git status",
            "git diff",
            "git log",
            "git show",
            "git add",
            "git commit",
            "git branch",
            "git switch",
            "git rev-parse",
            "git ls-files",
            "git grep",
            "git blame",
            "git worktree list",
            "git fetch",
            // Building and testing.
            "cargo build",
            "cargo check",
            "cargo test",
            "cargo clippy",
            "cargo fmt",
            "go build",
            "go test",
            "go vet",
            "npm test",
            "npm run test",
            "npm run lint",
            "npm run build",
            "npm run typecheck",
            "bun test",
            "bunx tsc",
            "pnpm test",
            "yarn test",
            "pytest",
            "make test",
        ]),
        deny: list(&[
            "git push",
            "git stash",
            "git reset --hard",
            "git clean",
            "git checkout --",
            "git restore",
            "rm -r",
            "rm -R",
            "env",
            "printenv",
        ]),
        ..Default::default()
    };
    // A reviewer may fetch, check out the commit it reviews and push it to the team branch,
    // never by force.
    let reviewer = PolicyConfig {
        extends: Some(BUILDER.into()),
        allow: list(&["git checkout --detach", "git push"]),
        deny: list(&[
            "git push --force",
            "git push -f",
            "git push --force-with-lease",
            "git push --mirror",
            "git push --delete",
        ]),
        deny_remove: list(&["git push"]),
        ..Default::default()
    };
    BTreeMap::from([(BUILDER.into(), builder), (REVIEWER.into(), reviewer)])
}

/// Resolves policy `name` against the built-ins and the owner's `configured` ones (which
/// replace a built-in of the same name). `None` for [`NONE`].
pub fn resolve(
    name: &str,
    configured: &BTreeMap<String, PolicyConfig>,
) -> Result<Option<Policy>, String> {
    let name = name.trim();
    if name.is_empty() || name == NONE {
        return Ok(None);
    }
    let builtin = builtins();
    let lookup = |n: &str| configured.get(n).or_else(|| builtin.get(n)).cloned();
    let mut chain = Vec::new();
    let mut next = Some(name.to_string());
    while let Some(n) = next {
        if chain.iter().any(|(c, _): &(String, PolicyConfig)| c == &n) {
            return Err(format!(
                "command policy '{name}' extends itself through '{n}'"
            ));
        }
        if chain.len() >= MAX_EXTENDS {
            return Err(format!(
                "command policy '{name}' extends more than {MAX_EXTENDS} levels"
            ));
        }
        let config = lookup(&n).ok_or_else(|| {
            let mut known: Vec<String> = builtin.keys().chain(configured.keys()).cloned().collect();
            known.sort();
            known.dedup();
            format!(
                "no command policy '{n}'; the policies are {}, or none",
                known.join(", ")
            )
        })?;
        next = config.extends.clone();
        chain.push((n, config));
    }
    let (mut allow, mut deny) = (Vec::<String>::new(), Vec::<String>::new());
    let (mut otherwise, mut scripts) = (Otherwise::Escalate, Scripts::Escalate);
    for (_, config) in chain.iter().rev() {
        allow.retain(|a| !config.allow_remove.contains(a));
        deny.retain(|d| !config.deny_remove.contains(d));
        for a in &config.allow {
            if !allow.contains(a) {
                allow.push(a.clone());
            }
        }
        for d in &config.deny {
            if !deny.contains(d) {
                deny.push(d.clone());
            }
        }
        otherwise = config.otherwise.unwrap_or(otherwise);
        scripts = config.scripts.unwrap_or(scripts);
    }
    for p in allow.iter().chain(&deny) {
        Pattern::parse(p)
            .ok_or_else(|| format!("command policy '{name}': '{p}' is not a command"))?;
    }
    Ok(Some(Policy {
        name: name.to_string(),
        allow,
        deny,
        otherwise,
        scripts,
    }))
}

/// A pattern: the command's leading words, and flags that must appear after them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    words: Vec<String>,
    flags: Vec<String>,
}

impl Pattern {
    fn parse(text: &str) -> Option<Pattern> {
        let tokens = split_words(text)?;
        let mut words = Vec::new();
        let mut flags = Vec::new();
        for t in tokens {
            if t.starts_with('-') && t.len() > 1 {
                flags.push(t);
            } else if flags.is_empty() {
                words.push(t);
            } else {
                // Words after a flag (`git checkout -- file`) are not part of a pattern.
                return None;
            }
        }
        (!words.is_empty()).then_some(Pattern { words, flags })
    }

    fn matches(&self, cmd: &Command) -> bool {
        cmd.words.len() >= self.words.len()
            && cmd.words.iter().zip(&self.words).all(|(a, b)| a == b)
            && self
                .flags
                .iter()
                .all(|f| cmd.rest.iter().any(|t| flag_matches(f, t)))
    }

    /// Whether every command this matches, `other` matches too.
    fn within(&self, other: &Pattern) -> bool {
        self.words.len() >= other.words.len()
            && self.words.iter().zip(&other.words).all(|(a, b)| a == b)
            && other
                .flags
                .iter()
                .all(|f| self.flags.iter().any(|g| flag_matches(f, g)))
    }
}

/// `--force` matches only itself (or `--force=x`); a short flag matches inside a cluster.
fn flag_matches(pattern: &str, token: &str) -> bool {
    if token == pattern || pattern == "--" && token == "--" {
        return true;
    }
    if let Some(long) = pattern.strip_prefix("--") {
        return token
            .strip_prefix("--")
            .is_some_and(|t| t == long || t.starts_with(&format!("{long}=")));
    }
    match (pattern.strip_prefix('-'), token.strip_prefix('-')) {
        (Some(p), Some(t)) if !t.starts_with('-') && !t.is_empty() => {
            t.chars().all(|c| c.is_ascii_alphanumeric()) && p.chars().all(|c| t.contains(c))
        }
        _ => false,
    }
}

/// One command of a command line: its leading words and what follows the first flag.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Command {
    words: Vec<String>,
    rest: Vec<String>,
    /// Inline code: `sh -c`, `python3 -c`, `eval`.
    script: bool,
    /// Runs as another user.
    elevated: bool,
}

/// Git options that come before the subcommand, and whether each takes a value.
const GIT_GLOBAL: &[(&str, bool)] = &[
    ("-C", true),
    ("-c", true),
    ("--git-dir", true),
    ("--work-tree", true),
    ("--namespace", true),
    ("--exec-path", false),
    ("--no-pager", false),
    ("-P", false),
    ("--paginate", false),
    ("-p", false),
    ("--bare", false),
    ("--no-replace-objects", false),
];

/// Wrappers that run the command after them.
const WRAPPERS: &[&str] = &["command", "exec", "nohup", "time", "nice", "builtin"];
const ELEVATE: &[&str] = &["sudo", "doas", "su", "pkexec"];
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh"];
const INTERPRETERS: &[&str] = &[
    "python",
    "python2",
    "python3",
    "node",
    "deno",
    "bun",
    "perl",
    "ruby",
    "php",
    "lua",
    "osascript",
];

fn is_assignment(t: &str) -> bool {
    t.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty()
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

fn is_redirect(t: &str) -> bool {
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
    t.starts_with('>') || t.starts_with('<')
}

impl Command {
    fn from_tokens(tokens: Vec<String>) -> Option<Command> {
        let mut tokens: Vec<String> = {
            let mut out = Vec::new();
            let mut skip_next = false;
            for t in tokens {
                if skip_next {
                    skip_next = false;
                    continue;
                }
                if is_redirect(&t) {
                    // `> file`: the operator alone takes the next token as its target.
                    skip_next = t
                        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '&')
                        .trim_start_matches(['>', '<', '&'])
                        .is_empty();
                    continue;
                }
                out.push(t);
            }
            out
        };
        let mut elevated = false;
        loop {
            while tokens.first().is_some_and(|t| is_assignment(t)) {
                tokens.remove(0);
            }
            let first = tokens.first().cloned()?;
            let base = first.rsplit('/').next().unwrap_or(&first).to_string();
            if ELEVATE.contains(&base.as_str()) {
                elevated = true;
                tokens.remove(0);
                while tokens.first().is_some_and(|t| t.starts_with('-')) {
                    tokens.remove(0);
                }
                continue;
            }
            if WRAPPERS.contains(&base.as_str())
                || (base == "env" && tokens.len() > 1)
                || base == "timeout"
            {
                tokens.remove(0);
                while tokens
                    .first()
                    .is_some_and(|t| t.starts_with('-') || is_assignment(t))
                {
                    tokens.remove(0);
                }
                if base == "timeout" && !tokens.is_empty() {
                    tokens.remove(0);
                }
                continue;
            }
            tokens[0] = base;
            break;
        }
        // Git's own options before its subcommand do not hide it.
        if tokens[0] == "git" {
            let i = 1;
            while i < tokens.len() {
                let t = tokens[i].clone();
                let known = GIT_GLOBAL.iter().find(|(o, _)| {
                    t == *o || (o.starts_with("--") && t.starts_with(&format!("{o}=")))
                });
                match known {
                    Some((o, takes)) => {
                        let inline = t.len() > o.len();
                        tokens.remove(i);
                        if *takes && !inline && i < tokens.len() {
                            tokens.remove(i);
                        }
                    }
                    None => break,
                }
            }
        }
        let split = tokens
            .iter()
            .position(|t| t.starts_with('-'))
            .unwrap_or(tokens.len());
        let rest = tokens.split_off(split);
        let words = tokens;
        let head = words[0].as_str();
        let script = head == "eval"
            || head == "source"
            || head == "."
            || (SHELLS.contains(&head) && rest.iter().any(|t| flag_matches("-c", t)))
            || (INTERPRETERS.contains(&head)
                && rest
                    .iter()
                    .any(|t| flag_matches("-c", t) || flag_matches("-e", t) || t == "--eval"));
        Some(Command {
            words,
            rest,
            script,
            elevated,
        })
    }
}

/// Splits text into words the way a shell would, honouring quotes and backslashes. `None`
/// for an unbalanced quote.
fn split_words(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        ch => cur.push(ch),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => cur.push(chars.next()?),
                        ch => cur.push(ch),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(ch) = chars.next() {
                    cur.push(ch);
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Some(words)
}

/// What a command line holds: its commands, and whether it hides more (substitution, a
/// heredoc, a subshell) that the words alone do not show.
struct Line {
    commands: Vec<Command>,
    hidden: bool,
}

/// Splits a command line into its commands. `None` when it cannot be read (an unbalanced
/// quote), which escalates.
fn parse_line(text: &str) -> Option<Line> {
    let mut segments: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut hidden = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some('\'') => {
                cur.push(c);
                if c == '\'' {
                    quote = None;
                }
                continue;
            }
            Some(q) => {
                cur.push(c);
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                    }
                } else if c == q {
                    quote = None;
                } else if c == '`' || (c == '$' && chars.peek() == Some(&'(')) {
                    hidden = true;
                }
                continue;
            }
            None => {}
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                cur.push(c);
            }
            '\\' => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '`' => hidden = true,
            '$' if chars.peek() == Some(&'(') => hidden = true,
            '<' if chars.peek() == Some(&'<') || chars.peek() == Some(&'(') => {
                hidden = true;
                cur.push(c);
            }
            '>' if chars.peek() == Some(&'(') => hidden = true,
            ';' | '\n' | '(' | ')' | '{' | '}' => segments.push(std::mem::take(&mut cur)),
            '|' | '&' => {
                // `2>&1` and `&>` are redirections, not separators.
                let redirect = cur.ends_with('>') || chars.peek() == Some(&'>');
                if redirect {
                    cur.push(c);
                } else {
                    if chars.peek() == Some(&c) {
                        chars.next();
                    }
                    segments.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    segments.push(cur);
    let mut commands = Vec::new();
    for s in segments {
        let words = split_words(&s)?;
        if let Some(cmd) = Command::from_tokens(words) {
            commands.push(cmd);
        }
    }
    Some(Line { commands, hidden })
}

impl Policy {
    fn patterns(list: &[String]) -> Vec<(String, Pattern)> {
        list.iter()
            .filter_map(|p| Pattern::parse(p).map(|pat| (p.clone(), pat)))
            .collect()
    }

    /// Judges one command line.
    pub fn decide(&self, command: &str) -> Decision {
        let text = command.trim();
        if text.is_empty() {
            return Decision::Escalate {
                reason: "the command is empty".into(),
            };
        }
        if text.len() > MAX_COMMAND {
            return Decision::Escalate {
                reason: "the command is too long to judge".into(),
            };
        }
        let Some(line) = parse_line(text) else {
            return Decision::Escalate {
                reason: "the command could not be read (an unbalanced quote)".into(),
            };
        };
        if line.commands.is_empty() {
            return Decision::Escalate {
                reason: "no command was found in it".into(),
            };
        }
        let (deny, allow) = (Self::patterns(&self.deny), Self::patterns(&self.allow));
        // A deny anywhere wins, whatever else the line holds.
        for cmd in &line.commands {
            if let Some((rule, _)) = deny.iter().find(|(_, p)| p.matches(cmd)) {
                return Decision::Deny { rule: rule.clone() };
            }
        }
        if line.commands.iter().any(|c| c.elevated) {
            return Decision::Escalate {
                reason: "it runs a command as another user".into(),
            };
        }
        if self.scripts == Scripts::Escalate {
            if line.hidden {
                return Decision::Escalate {
                    reason:
                        "it runs code the policy cannot see (command substitution or a heredoc)"
                            .into(),
                };
            }
            if let Some(cmd) = line.commands.iter().find(|c| c.script) {
                return Decision::Escalate {
                    reason: format!("it runs inline code with {}", cmd.words[0]),
                };
            }
        }
        let mut rules = Vec::new();
        for cmd in &line.commands {
            match allow.iter().find(|(_, p)| p.matches(cmd)) {
                Some((rule, _)) => rules.push(rule.clone()),
                None => match self.otherwise {
                    Otherwise::Allow => rules.push("otherwise: allow".into()),
                    Otherwise::Deny => {
                        return Decision::Deny {
                            rule: format!(
                                "otherwise: deny ({} is not allowed)",
                                cmd.words.join(" ")
                            ),
                        };
                    }
                    Otherwise::Escalate => {
                        return Decision::Escalate {
                            reason: format!("the policy does not name {}", cmd.words.join(" ")),
                        };
                    }
                },
            }
        }
        rules.dedup();
        Decision::Allow {
            rule: rules.join(", "),
        }
    }

    /// Judges a file edit or a path: inside the session's folder is allowed, anything else
    /// goes to the owner.
    pub fn decide_path(&self, path: &str, folder: &str) -> Decision {
        let inside = within(path.trim(), folder);
        match inside {
            true => Decision::Allow {
                rule: "inside the session's folder".into(),
            },
            false => Decision::Escalate {
                reason: "it is outside the session's folder".into(),
            },
        }
    }

    /// Whether everything this policy lets through, `wider` lets through too: a session may
    /// start a worker only on a policy no wider than its own.
    pub fn no_wider_than(&self, wider: &Policy) -> bool {
        let (my_allow, my_deny) = (Self::patterns(&self.allow), Self::patterns(&self.deny));
        let (their_allow, their_deny) = (Self::patterns(&wider.allow), Self::patterns(&wider.deny));
        self.otherwise <= wider.otherwise
            && self.scripts <= wider.scripts
            && my_allow
                .iter()
                .all(|(_, a)| their_allow.iter().any(|(_, b)| a.within(b)))
            && their_deny
                .iter()
                .all(|(_, d)| my_deny.iter().any(|(_, e)| d.within(e)))
    }

    /// The commands to deny when the agent starts, for a CLI whose rules name a command and
    /// its subcommand but no flags (Copilot's `--deny-tool=shell(...)`). A deny with a flag
    /// becomes the whole command, unless the policy allows that command, in which case it is
    /// left to the dialogs the agent still shows.
    pub fn launch_denies(&self) -> Vec<String> {
        let allow = Self::patterns(&self.allow);
        let mut out: Vec<String> = Vec::new();
        for (_, p) in Self::patterns(&self.deny) {
            let words: Vec<String> = p.words.iter().take(2).cloned().collect();
            let allowed = allow
                .iter()
                .any(|(_, a)| a.words.len() >= words.len() && a.words[..words.len()] == words[..]);
            let joined = words.join(" ");
            if !(allowed && !p.flags.is_empty()) && !out.contains(&joined) {
                out.push(joined);
            }
        }
        out
    }
}

/// Lexically normalised `path` (relative to `folder` when not absolute) is `folder` or below
/// it. A `..` that leaves `folder` is outside.
fn within(path: &str, folder: &str) -> bool {
    if path.is_empty() || folder.is_empty() {
        return false;
    }
    let base = normalise(Path::new(folder));
    let joined = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        Path::new(folder).join(path)
    };
    let Some(base) = base else { return false };
    normalise(&joined).is_some_and(|p| p.starts_with(&base))
}

fn normalise(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(name: &str) -> Policy {
        resolve(name, &BTreeMap::new()).unwrap().unwrap()
    }

    fn decide(p: &Policy, cmd: &str) -> &'static str {
        p.decide(cmd).as_str()
    }

    #[test]
    fn the_builder_allows_its_tree_and_denies_what_reaches_past_it() {
        let b = policy(BUILDER);
        for ok in [
            "git status",
            "git add -A && git commit -m 'parser: handle tabs'",
            "cargo test -p hq-core 2>&1 | tail -20",
            "FOO=1 cargo check",
            "git -C /work/app diff --stat",
            "ls -la src",
        ] {
            assert_eq!(decide(&b, ok), "allow", "{ok}");
        }
        for (cmd, rule) in [
            ("git push origin main", "git push"),
            ("git -C /work/app push", "git push"),
            ("cargo test && git push", "git push"),
            ("git stash", "git stash"),
            ("git reset --hard HEAD~1", "git reset --hard"),
            ("rm -rf target", "rm -r"),
            ("rm -fr target", "rm -r"),
            ("rm -R target", "rm -R"),
            ("env", "env"),
            ("/usr/bin/env", "env"),
            ("printenv HOME", "printenv"),
            ("git checkout -- src/main.rs", "git checkout --"),
            ("echo hi; git clean -fdx", "git clean"),
            ("sudo git push", "git push"),
        ] {
            assert_eq!(b.decide(cmd), Decision::Deny { rule: rule.into() }, "{cmd}");
        }
        for unknown in ["curl https://example.com", "npx some-tool", "rm file.txt"] {
            assert_eq!(decide(&b, unknown), "escalate", "{unknown}");
        }
        assert_eq!(
            decide(&b, "git reset HEAD file"),
            "escalate",
            "only --hard is denied"
        );
    }

    #[test]
    fn scripts_and_hidden_code_go_to_the_owner_unless_allowed() {
        let b = policy(BUILDER);
        for hidden in [
            "echo $(cat secret)",
            "echo `whoami`",
            "bash -c 'git push'",
            "sh -c ls",
            "python3 -c 'import os'",
            "node -e 'process.exit(0)'",
            "eval ls",
            "cat <<EOF > x\nhi\nEOF",
            "diff <(ls a) <(ls b)",
            "sudo ls",
        ] {
            assert_eq!(decide(&b, hidden), "escalate", "{hidden}");
        }
        assert_eq!(decide(&b, "echo 'not $(hidden) in single quotes'"), "allow");
        let mut open = b.clone();
        open.scripts = Scripts::Allow;
        open.otherwise = Otherwise::Allow;
        assert_eq!(decide(&open, "bash -c 'make'"), "allow");
        assert_eq!(
            decide(&open, "bash -c 'x'; git push"),
            "deny",
            "a deny still wins"
        );
    }

    #[test]
    fn a_reviewer_may_push_a_commit_but_never_by_force() {
        let r = policy(REVIEWER);
        assert_eq!(
            decide(&r, "git push origin abc123:refs/heads/team"),
            "allow"
        );
        assert_eq!(decide(&r, "git checkout --detach abc123"), "allow");
        for force in [
            "git push --force origin main",
            "git push origin main --force",
            "git push -f",
            "git push --force-with-lease",
            "git push origin --delete team",
        ] {
            assert_eq!(decide(&r, force), "deny", "{force}");
        }
        assert_eq!(
            decide(&r, "git stash"),
            "deny",
            "inherited from the builder"
        );
    }

    #[test]
    fn policies_resolve_through_extends_and_the_owners_entries_win() {
        let configured = BTreeMap::from([
            (
                BUILDER.to_string(),
                PolicyConfig {
                    allow: vec!["git status".into(), "pnpm codegen".into()],
                    deny: vec!["pnpm deploy".into()],
                    otherwise: Some(Otherwise::Deny),
                    ..Default::default()
                },
            ),
            (
                "strict".to_string(),
                PolicyConfig {
                    extends: Some(BUILDER.into()),
                    allow_remove: vec!["pnpm codegen".into()],
                    ..Default::default()
                },
            ),
            (
                "loop".to_string(),
                PolicyConfig {
                    extends: Some("loop".into()),
                    ..Default::default()
                },
            ),
        ]);
        let b = resolve(BUILDER, &configured).unwrap().unwrap();
        assert_eq!(
            b.allow,
            ["git status", "pnpm codegen"],
            "the owner's builder replaces ours"
        );
        assert_eq!(decide(&b, "ls"), "deny", "otherwise: deny");
        let s = resolve("strict", &configured).unwrap().unwrap();
        assert_eq!(s.allow, ["git status"]);
        assert_eq!(s.otherwise, Otherwise::Deny, "inherited");
        assert!(
            resolve("loop", &configured)
                .unwrap_err()
                .contains("extends itself")
        );
        assert!(
            resolve("nope", &configured)
                .unwrap_err()
                .contains("builder, loop, reviewer, strict")
        );
        assert_eq!(resolve(NONE, &configured).unwrap(), None);
        let bad = BTreeMap::from([(
            "bad".to_string(),
            PolicyConfig {
                deny: vec!["git checkout -- file".into()],
                ..Default::default()
            },
        )]);
        assert!(
            resolve("bad", &bad).is_err(),
            "words after a flag are not a pattern"
        );
    }

    #[test]
    fn a_worker_may_only_take_a_policy_no_wider_than_its_own() {
        let (b, r) = (policy(BUILDER), policy(REVIEWER));
        assert!(b.no_wider_than(&b));
        assert!(!r.no_wider_than(&b), "the reviewer may push");
        assert!(
            b.no_wider_than(&r),
            "the builder denies every push, so it is narrower than the reviewer"
        );
        let mut narrow = b.clone();
        narrow.allow.retain(|a| a.starts_with("git"));
        narrow.deny.push("cargo publish".into());
        assert!(narrow.no_wider_than(&b));
        let mut loose = b.clone();
        loose.otherwise = Otherwise::Allow;
        assert!(!loose.no_wider_than(&b));
        let mut fewer_denies = b.clone();
        fewer_denies.deny.retain(|d| d != "git push");
        assert!(!fewer_denies.no_wider_than(&b));
    }

    #[test]
    fn launch_denies_name_commands_without_flags() {
        let b = policy(BUILDER);
        let d = b.launch_denies();
        assert!(d.contains(&"git push".to_string()));
        assert!(
            d.contains(&"git reset".to_string()),
            "a flag widens to the whole command"
        );
        assert!(d.contains(&"rm".to_string()));
        assert!(!d.iter().any(|c| c.contains('-')), "{d:?}");
        let r = policy(REVIEWER);
        assert!(
            !r.launch_denies().contains(&"git push".to_string()),
            "the reviewer may push, so force stays with the dialogs"
        );
    }

    #[test]
    fn paths_inside_the_folder_are_allowed_and_the_rest_escalate() {
        let b = policy(BUILDER);
        assert_eq!(
            b.decide_path("/w/app/src/lib.rs", "/w/app").as_str(),
            "allow"
        );
        assert_eq!(b.decide_path("src/lib.rs", "/w/app").as_str(), "allow");
        assert_eq!(
            b.decide_path("/w/app/../other", "/w/app").as_str(),
            "escalate"
        );
        assert_eq!(
            b.decide_path("../../etc/passwd", "/w/app").as_str(),
            "escalate"
        );
        assert_eq!(
            b.decide_path("/w/application", "/w/app").as_str(),
            "escalate"
        );
        assert_eq!(b.decide_path("", "/w/app").as_str(), "escalate");
    }

    #[test]
    fn odd_lines_escalate_rather_than_pass() {
        let b = policy(BUILDER);
        assert_eq!(decide(&b, ""), "escalate");
        assert_eq!(decide(&b, "echo 'unclosed"), "escalate");
        assert_eq!(decide(&b, &"ls ".repeat(10_000)), "escalate");
        assert_eq!(decide(&b, "timeout 5 git push"), "deny");
        assert_eq!(decide(&b, "nohup cargo test &"), "allow");
    }
}
