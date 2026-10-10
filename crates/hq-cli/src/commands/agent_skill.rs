//! `hq task install-skill`: puts the `hq-tasks` skill where each coding agent looks for
//! skills, so an agent that connects to HQ knows how to work its tasks. The file is the
//! same text for every agent, written the way that agent reads it, and updated in place
//! when HQ updates. A file that does not carry HQ's marker, is a symlink, or cannot be read as
//! text is not HQ's and is left alone unless `--force` says otherwise; a copy that still carries
//! the marker is treated as HQ's and rewritten, so edit a copy under another name to keep changes.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

pub const SKILL_NAME: &str = "hq-tasks";
const SKILL_SOURCE: &str = include_str!("../../assets/skills/hq-tasks/SKILL.md");
/// Present in every file HQ wrote, so a later run knows it may rewrite it.
const MANAGED_MARKER: &str = "<!-- managed by hq: `hq tasks install-skill` rewrites this file when HQ updates; edit a copy to keep changes -->";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
    Cursor,
}

impl Harness {
    pub const ALL: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Cursor];

    pub fn name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Cursor => "cursor",
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|h| h.name() == text.to_ascii_lowercase())
            .ok_or_else(|| anyhow::anyhow!("unknown agent '{text}', expected one of claude, codex, cursor, or all"))
    }
}

/// The configuration directory of each supported agent.
#[derive(Clone, Debug)]
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub cursor: PathBuf,
}

impl Roots {
    /// Where each agent keeps its configuration on this machine, honouring the variables the
    /// agents themselves read (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`).
    pub fn from_env() -> Option<Self> {
        let home = dirs::home_dir()?;
        let from_var = |var: &str, fallback: &str| {
            std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home.join(fallback))
        };
        Some(Self {
            claude: from_var("CLAUDE_CONFIG_DIR", ".claude"),
            codex: from_var("CODEX_HOME", ".codex"),
            cursor: home.join(".cursor"),
        })
    }

    fn root(&self, harness: Harness) -> &Path {
        match harness {
            Harness::Claude => &self.claude,
            Harness::Codex => &self.codex,
            Harness::Cursor => &self.cursor,
        }
    }

    pub fn target(&self, harness: Harness) -> PathBuf {
        let root = self.root(harness);
        match harness {
            Harness::Claude | Harness::Codex => root.join("skills").join(SKILL_NAME).join("SKILL.md"),
            Harness::Cursor => root.join("rules").join(format!("{SKILL_NAME}.mdc")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    Updated,
    Unchanged,
    /// A file there is not HQ's (or was edited); left as it is.
    SkippedEdited,
    /// The agent's configuration directory is not on this machine.
    NotFound,
}

/// The skill's own frontmatter and its body, whatever line endings the checkout used.
fn split_source() -> (String, String) {
    let source = SKILL_SOURCE.replace("\r\n", "\n");
    let rest = source.strip_prefix("---\n").unwrap_or(&source);
    match rest.split_once("\n---\n") {
        Some((front, body)) => (front.to_string(), body.trim_start_matches('\n').to_string()),
        None => (String::new(), source.clone()),
    }
}

fn description() -> String {
    let (front, _) = split_source();
    front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(|d| d.trim().to_string())
        .unwrap_or_default()
}

/// The file's text for `harness`.
pub fn rendered(harness: Harness) -> String {
    let (front, body) = split_source();
    let version = env!("CARGO_PKG_VERSION");
    match harness {
        Harness::Claude | Harness::Codex => {
            format!("---\n{front}\n---\n{MANAGED_MARKER}\n<!-- hq {version} -->\n\n{body}")
        }
        Harness::Cursor => format!(
            "---\ndescription: {}\nalwaysApply: false\n---\n{MANAGED_MARKER}\n<!-- hq {version} -->\n\n{body}",
            description()
        ),
    }
}

/// Writes through a temporary file in the same directory, so a failure never leaves half a file.
fn write(path: &Path, text: &str) -> Result<()> {
    let Some(parent) = path.parent() else {
        bail!("{} has no directory", path.display());
    };
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{SKILL_NAME}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// What is at `path` today, as far as HQ may act on it.
enum Existing {
    Missing,
    /// HQ's own file, with this text.
    Managed(String),
    /// Anything else: someone else's text, a symlink, or something unreadable.
    NotHqs,
}

fn existing_at(path: &Path) -> Result<Existing> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Existing::Missing),
        Err(e) => return Err(e.into()),
    };
    // Writing through a link would change a file somewhere else.
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Ok(Existing::NotHqs);
    }
    Ok(match std::fs::read(path).ok().and_then(|b| String::from_utf8(b).ok()) {
        Some(text) if text.contains(MANAGED_MARKER) => Existing::Managed(text),
        _ => Existing::NotHqs,
    })
}

fn install_one(roots: &Roots, harness: Harness, create_root: bool, dry_run: bool, force: bool) -> Result<(PathBuf, Outcome)> {
    let target = roots.target(harness);
    if !create_root && !roots.root(harness).is_dir() {
        return Ok((target, Outcome::NotFound));
    }
    let wanted = rendered(harness);
    let outcome = match existing_at(&target)? {
        Existing::Missing => Outcome::Installed,
        Existing::Managed(text) if text == wanted => Outcome::Unchanged,
        Existing::Managed(_) => Outcome::Updated,
        Existing::NotHqs if force && !is_link(&target) => Outcome::Updated,
        Existing::NotHqs => Outcome::SkippedEdited,
    };
    if !dry_run && matches!(outcome, Outcome::Installed | Outcome::Updated) {
        write(&target, &wanted)?;
    }
    Ok((target, outcome))
}

/// Whether `path` is a symlink. `--force` still never writes through one.
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Installs the skill for the named agents (all of them when `wanted` is empty), writing
/// nothing under `dry_run`. An agent named on purpose gets its directory made; with none
/// named only agents found on this machine are touched.
pub fn install(roots: &Roots, wanted: &[Harness], dry_run: bool, force: bool) -> Result<Vec<(Harness, PathBuf, Outcome)>> {
    let named = !wanted.is_empty();
    let targets: Vec<Harness> = if named { wanted.to_vec() } else { Harness::ALL.to_vec() };
    let mut out = Vec::new();
    for harness in targets {
        let (path, outcome) = install_one(roots, harness, named, dry_run, force)?;
        out.push((harness, path, outcome));
    }
    Ok(out)
}

/// Runs the command: parses the agent names, installs, and prints one line each.
pub fn run(args: &[String], dry_run: bool, force: bool, out: &mut dyn std::io::Write) -> Result<()> {
    let wanted: Vec<Harness> = match args {
        [] => Vec::new(),
        [all] if all == "all" => Vec::new(),
        names => names.iter().map(|n| Harness::parse(n)).collect::<Result<_>>()?,
    };
    let Some(roots) = Roots::from_env() else {
        bail!("cannot find your home directory");
    };
    report(&install(&roots, &wanted, dry_run, force)?, dry_run, out)
}

fn report(results: &[(Harness, PathBuf, Outcome)], dry_run: bool, out: &mut dyn std::io::Write) -> Result<()> {
    let prefix = if dry_run { "would " } else { "" };
    for (harness, path, outcome) in results {
        let line = match outcome {
            Outcome::Installed => format!("{prefix}install {} ({})", path.display(), harness.name()),
            Outcome::Updated => format!("{prefix}update {} ({})", path.display(), harness.name()),
            Outcome::Unchanged => format!("{} is up to date ({})", path.display(), harness.name()),
            Outcome::SkippedEdited => format!(
                "{} exists and is not HQ's copy, left alone ({}); pass --force to replace it",
                path.display(),
                harness.name()
            ),
            Outcome::NotFound => format!("{}: not found on this machine, skipped", harness.name()),
        };
        writeln!(out, "{line}")?;
    }
    if results.iter().all(|(_, _, o)| *o == Outcome::NotFound) {
        writeln!(out, "No supported agent was found. Name one to install anyway: hq task install-skill claude")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> Roots {
        Roots { claude: dir.join(".claude"), codex: dir.join(".codex"), cursor: dir.join(".cursor") }
    }

    #[test]
    fn the_skill_is_generic_and_valid() {
        let text = SKILL_SOURCE;
        // Spelled in pieces so this file does not itself trip the personal-data scan.
        let private_words = [["cal", "vin"], ["mage", "zi"], ["kola", "borate"], ["/Us", "ers/"], ["out", "look"]];
        for [head, tail] in private_words {
            let word = format!("{head}{tail}");
            assert!(!text.to_lowercase().contains(&word.to_lowercase()), "the skill names a private word");
        }
        assert!(!text.contains('\u{2014}'), "no em dashes in prose");
        for tool in ["task_next", "task_claim", "task_heartbeat", "task_release", "task_create_many", "task_link_list", "task_stale", "task_restore"] {
            assert!(text.contains(tool), "{tool}");
        }
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        std::fs::create_dir_all(skills.join(SKILL_NAME)).unwrap();
        std::fs::write(skills.join(SKILL_NAME).join("SKILL.md"), rendered(Harness::Claude)).unwrap();
        let issues = hq_tools::skills::validate_skills(&skills);
        assert!(issues.iter().all(|i| i.severity != hq_tools::skills::Severity::Error), "{issues:?}");
    }

    #[test]
    fn it_installs_where_an_agent_is_found_and_skips_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        let r = install(&roots(dir.path()), &[], false, false).unwrap();
        let outcome = |h: Harness| r.iter().find(|(x, _, _)| *x == h).unwrap().2.clone();
        assert_eq!(outcome(Harness::Claude), Outcome::Installed);
        assert_eq!(outcome(Harness::Codex), Outcome::NotFound);
        assert_eq!(outcome(Harness::Cursor), Outcome::NotFound);
        let written = std::fs::read_to_string(roots(dir.path()).target(Harness::Claude)).unwrap();
        assert!(written.starts_with("---\nname: hq-tasks"));
        assert!(written.contains(MANAGED_MARKER));
        assert!(!dir.path().join(".codex").exists(), "an agent that is not there is not created");
    }

    #[test]
    fn naming_an_agent_creates_its_directory_and_cursor_gets_a_rule_file() {
        let dir = tempfile::tempdir().unwrap();
        let r = install(&roots(dir.path()), &[Harness::Cursor], false, false).unwrap();
        assert_eq!(r[0].2, Outcome::Installed);
        let path = roots(dir.path()).target(Harness::Cursor);
        assert!(path.ends_with("rules/hq-tasks.mdc"));
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.starts_with("---\ndescription: Work HQ tasks properly."));
        assert!(text.contains("alwaysApply: false"));
        assert!(!text.contains("name: hq-tasks"), "the rule file carries cursor's frontmatter, not the skill's");
    }

    #[test]
    fn running_twice_changes_nothing_and_a_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let dry = install(&roots, &[Harness::Claude], true, false).unwrap();
        assert_eq!(dry[0].2, Outcome::Installed);
        assert!(!roots.target(Harness::Claude).exists());
        install(&roots, &[Harness::Claude], false, false).unwrap();
        assert_eq!(install(&roots, &[Harness::Claude], false, false).unwrap()[0].2, Outcome::Unchanged);
    }

    #[test]
    fn an_older_hq_copy_is_updated_but_a_copy_a_person_wrote_is_left_alone_unless_forced() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let path = roots.target(Harness::Claude);
        write(&path, &format!("---\nname: hq-tasks\n---\n{MANAGED_MARKER}\nold text\n")).unwrap();
        assert_eq!(install(&roots, &[Harness::Claude], false, false).unwrap()[0].2, Outcome::Updated);
        assert!(std::fs::read_to_string(&path).unwrap().contains("# Working HQ tasks"));

        write(&path, "---\nname: hq-tasks\n---\nmy own version\n").unwrap();
        assert_eq!(install(&roots, &[Harness::Claude], false, false).unwrap()[0].2, Outcome::SkippedEdited);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "---\nname: hq-tasks\n---\nmy own version\n");
        assert_eq!(install(&roots, &[Harness::Claude], false, true).unwrap()[0].2, Outcome::Updated);
        assert!(std::fs::read_to_string(&path).unwrap().contains("# Working HQ tasks"));
    }

    #[test]
    fn a_symlink_is_never_written_through_even_with_force_and_an_unreadable_file_does_not_stop_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, format!("precious {MANAGED_MARKER}")).unwrap();
        let path = roots.target(Harness::Claude);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&victim, &path).unwrap();
            for force in [false, true] {
                assert_eq!(install(&roots, &[Harness::Claude], false, force).unwrap()[0].2, Outcome::SkippedEdited);
            }
            assert_eq!(std::fs::read_to_string(&victim).unwrap(), format!("precious {MANAGED_MARKER}"));
        }
        let codex = roots.target(Harness::Codex);
        std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
        std::fs::write(&codex, [0xff, 0xfe, 0x00]).unwrap();
        let r = install(&roots, &[Harness::Codex, Harness::Cursor], false, false).unwrap();
        assert_eq!(r[0].2, Outcome::SkippedEdited, "not text, so not HQ's");
        assert_eq!(r[1].2, Outcome::Installed, "the next agent still gets its copy");
    }

    #[test]
    fn a_file_that_only_mentions_the_phrase_is_not_taken_for_hqs() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let path = roots.target(Harness::Claude);
        write(&path, "my notes: this is not managed by hq: at all\n").unwrap();
        assert_eq!(install(&roots, &[Harness::Claude], false, false).unwrap()[0].2, Outcome::SkippedEdited);
    }

    #[test]
    fn names_parse_and_an_unknown_one_is_an_error_naming_the_choices() {
        assert_eq!(Harness::parse("Codex").unwrap(), Harness::Codex);
        let err = Harness::parse("emacs").unwrap_err().to_string();
        assert!(err.contains("claude") && err.contains("codex") && err.contains("cursor"), "{err}");
    }

    #[test]
    fn the_report_says_what_happened_and_what_to_do_when_nothing_was_found() {
        let dir = tempfile::tempdir().unwrap();
        let results = install(&roots(dir.path()), &[], false, false).unwrap();
        let mut out = Vec::new();
        report(&results, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("not found on this machine"), "{text}");
        assert!(text.contains("hq task install-skill claude"), "{text}");
    }
}
