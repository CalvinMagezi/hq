//! `hq onboard --from claude --dry-run`: read an existing coding agent's state and print what HQ
//! would set up from it (spaces, initiatives, parked projects, preferences, briefs). Read-only:
//! transcripts are counted and dated, never opened, and settings are reported by name only, so
//! no value under `env`, a token or a key is read into the output. Nothing is written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Result, bail};
use serde_json::Value;

/// A project with a session this recent counts as active; older ones are proposed as parked.
const ACTIVE_WITHIN: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const SECS_PER_DAY: u64 = 24 * 60 * 60;
const BYTES_PER_MB: u64 = 1024 * 1024;
/// Path components a slug decode may try before giving up, so a pathological name stays cheap.
const MAX_DECODE_STEPS: usize = 4096;

/// One project folder under `~/.claude/projects`.
#[derive(Debug)]
struct Project {
    slug: String,
    /// The repository the slug names, when it could be matched to a folder that exists.
    repo: Option<PathBuf>,
    sessions: usize,
    last_active: Option<SystemTime>,
    bytes: u64,
    memory_notes: usize,
}

impl Project {
    fn is_active(&self, now: SystemTime) -> bool {
        self.repo.is_some()
            && self
                .last_active
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age <= ACTIVE_WITHIN)
    }
}

/// Turn a project slug back into the folder it names. Claude Code writes the path with every
/// `/` and `.` as `-`, so the slug is ambiguous; each reading is tried against the filesystem
/// and the first folder that exists wins.
fn decode_slug(slug: &str, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let parts: Vec<&str> = slug.trim_start_matches('-').split('-').collect();
    let mut steps = 0;
    decode_from(
        &parts,
        PathBuf::from("/"),
        String::new(),
        exists,
        &mut steps,
    )
}

fn decode_from(
    parts: &[&str],
    dir: PathBuf,
    component: String,
    exists: &dyn Fn(&Path) -> bool,
    steps: &mut usize,
) -> Option<PathBuf> {
    *steps += 1;
    if *steps > MAX_DECODE_STEPS {
        return None;
    }
    let Some((first, rest)) = parts.split_first() else {
        let full = dir.join(&component);
        return (!component.is_empty() && exists(&full)).then_some(full);
    };
    if component.is_empty() {
        // An empty part is the `.` of a hidden folder: `/.config` becomes `--config`.
        if first.is_empty() {
            let (next, rest) = rest.split_first()?;
            return decode_from(rest, dir, format!(".{next}"), exists, steps);
        }
        return decode_from(rest, dir, first.to_string(), exists, steps);
    }
    // Close this component as a folder and start the next one, or keep extending it.
    let closed = dir.join(&component);
    if exists(&closed)
        && let Some(found) = decode_from(parts, closed, String::new(), exists, steps)
    {
        return Some(found);
    }
    for joiner in ['-', '.'] {
        let longer = format!("{component}{joiner}{first}");
        if let Some(found) = decode_from(rest, dir.clone(), longer, exists, steps) {
            return Some(found);
        }
    }
    None
}

fn scan_project(dir: &Path) -> Project {
    let slug = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut project = Project {
        repo: decode_slug(&slug, &|p| p.is_dir()),
        slug,
        sessions: 0,
        last_active: None,
        bytes: 0,
        memory_notes: 0,
    };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if path.extension().is_some_and(|e| e == "jsonl") {
            project.sessions += 1;
            project.bytes += meta.len();
            let modified = meta.modified().ok();
            project.last_active = project.last_active.max(modified);
        }
    }
    project.memory_notes = std::fs::read_dir(dir.join("memory"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count();
    project
}

/// Settings reported by name: which model and modes the agent runs, and which plugins, hooks and
/// MCP servers exist. Values that could hold secrets (`env`, server configs) are never read out.
fn settings_lines(settings: &Value) -> Vec<String> {
    let text = |key: &str| settings.get(key).and_then(Value::as_str);
    let count = |key: &str| {
        settings
            .get(key)
            .and_then(Value::as_object)
            .map_or(0, |m| m.len())
    };
    let mut lines = Vec::new();
    for key in ["model", "effortLevel", "outputStyle"] {
        if let Some(value) = text(key) {
            lines.push(format!("{key}: {value}"));
        }
    }
    if let Some(mode) = settings
        .pointer("/permissions/defaultMode")
        .and_then(Value::as_str)
    {
        lines.push(format!("permission mode: {mode}"));
    }
    lines.push(format!(
        "{} plugins enabled, {} hook events configured",
        count("enabledPlugins"),
        count("hooks")
    ));
    if let Some(servers) = settings.get("mcpServers").and_then(Value::as_object) {
        let names: Vec<&str> = servers.keys().map(String::as_str).collect();
        lines.push(format!("MCP servers: {}", names.join(", ")));
    }
    lines
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

fn days_ago(t: Option<SystemTime>, now: SystemTime) -> String {
    match t.and_then(|t| now.duration_since(t).ok()) {
        Some(age) => format!("{}d ago", age.as_secs() / SECS_PER_DAY),
        None => "never".to_string(),
    }
}

/// The space a repository would go in: the folder that holds it, named by its last component.
fn space_of(repo: &Path) -> String {
    repo.parent().and_then(Path::file_name).map_or_else(
        || "Projects".to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

pub fn run(from: &str, dry_run: bool) -> Result<()> {
    if from != "claude" {
        bail!("only `--from claude` is supported so far; codex and cursor are planned");
    }
    if !dry_run {
        bail!(
            "importing is not written yet; run `hq onboard --from claude --dry-run` to see what it would set up"
        );
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let root = home.join(".claude");
    if !root.is_dir() {
        bail!(
            "{} does not exist, so there is nothing to import",
            root.display()
        );
    }
    let now = SystemTime::now();
    let mut projects: Vec<Project> = std::fs::read_dir(root.join("projects"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| scan_project(&e.path()))
        .collect();
    projects.sort_by(|a, b| b.sessions.cmp(&a.sessions));

    println!(
        "\nWhat HQ would set up from {} (dry run: nothing is written)\n",
        root.display()
    );

    let (active, parked): (Vec<&Project>, Vec<&Project>) =
        projects.iter().partition(|p| p.is_active(now));
    let mut spaces: BTreeMap<String, Vec<&Project>> = BTreeMap::new();
    for project in &active {
        if let Some(repo) = &project.repo {
            spaces.entry(space_of(repo)).or_default().push(project);
        }
    }
    println!(
        "Spaces and initiatives ({} active projects, a session in the last {} days)",
        active.len(),
        ACTIVE_WITHIN.as_secs() / SECS_PER_DAY
    );
    for (space, members) in &spaces {
        println!("  {space}");
        for p in members {
            let repo = p.repo.as_deref().unwrap_or(Path::new(""));
            let brief = if p.memory_notes > 0 {
                format!(", brief from {} memory notes", p.memory_notes)
            } else {
                String::new()
            };
            println!(
                "    {}  {} sessions, last {}, {} MB{brief}",
                repo.display(),
                p.sessions,
                days_ago(p.last_active, now),
                p.bytes / BYTES_PER_MB
            );
        }
    }

    println!(
        "\nParked ({}): older, or the folder no longer exists",
        parked.len()
    );
    for p in &parked {
        let name = p.repo.as_ref().map_or_else(
            || format!("{} (folder not found)", p.slug),
            |r| r.display().to_string(),
        );
        println!(
            "    {name}  {} sessions, last {}",
            p.sessions,
            days_ago(p.last_active, now)
        );
    }

    println!("\nPreferences and rules");
    let claude_md = root.join("CLAUDE.md");
    if claude_md.is_file() {
        let lines = std::fs::read_to_string(&claude_md).map_or(0, |t| t.lines().count());
        println!(
            "    {} ({lines} lines): source for PREFERENCES.md, linked rather than copied",
            claude_md.display()
        );
    }
    let briefs = active.iter().filter(|p| p.memory_notes > 0).count();
    println!("    {briefs} project briefs would link to that project's memory notes");

    if let Ok(text) = std::fs::read_to_string(root.join("settings.json"))
        && let Ok(settings) = serde_json::from_str::<Value>(&text)
    {
        println!("\nHow the agent is configured (left as it is; HQ does not restate it)");
        for line in settings_lines(&settings) {
            println!("    {line}");
        }
    }
    for (label, dir) in [("Skills", "skills"), ("Scheduled tasks", "scheduled-tasks")] {
        let names = names_in(&root.join(dir));
        if !names.is_empty() {
            println!("\n{label} ({}): {}", names.len(), names.join(", "));
        }
    }

    println!(
        "\nRead: folder listings, file sizes and dates, settings.json key names, CLAUDE.md length. \
         Not read: transcript contents, env values, credentials. Nothing under {} was changed.\n",
        root.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_slug_decodes_to_the_folder_that_exists() {
        let dirs = [
            "/Users",
            "/Users/me",
            "/Users/me/src",
            "/Users/me/src/my-app",
            "/Users/me/.config",
            "/Users/me/.config/hq.d",
        ];
        let exists = |p: &Path| dirs.iter().any(|d| Path::new(d) == p);
        assert_eq!(
            decode_slug("-Users-me-src-my-app", &exists),
            Some(PathBuf::from("/Users/me/src/my-app"))
        );
        assert_eq!(
            decode_slug("-Users-me--config-hq-d", &exists),
            Some(PathBuf::from("/Users/me/.config/hq.d"))
        );
        assert_eq!(decode_slug("-Users-me-gone", &exists), None);
    }

    #[test]
    fn settings_are_reported_by_name_without_secret_values() {
        let settings = json!({
            "model": "opus",
            "env": { "ANTHROPIC_API_KEY": "sk-secret" },
            "mcpServers": { "hq": { "headers": { "Authorization": "Bearer t0ken" } } },
            "permissions": { "defaultMode": "plan" }
        });
        let text = settings_lines(&settings).join("\n");
        assert!(text.contains("model: opus") && text.contains("MCP servers: hq"));
        assert!(text.contains("permission mode: plan"));
        assert!(!text.contains("sk-secret") && !text.contains("t0ken"));
    }

    #[test]
    fn only_a_recent_project_with_a_folder_is_active() {
        let now = SystemTime::now();
        let project = |repo: Option<&str>, age_days: u64| Project {
            slug: String::new(),
            repo: repo.map(PathBuf::from),
            sessions: 1,
            last_active: Some(now - Duration::from_secs(age_days * SECS_PER_DAY)),
            bytes: 0,
            memory_notes: 0,
        };
        assert!(project(Some("/r"), 3).is_active(now));
        assert!(!project(Some("/r"), 90).is_active(now));
        assert!(!project(None, 3).is_active(now));
    }
}
