//! "Use HQ in Claude": what Claude Code and Claude Desktop on this computer hold for each paired
//! hub, and adding or removing it. The work is `hq_bridge::claude`; this turns it into what the
//! control window shows. The key never leaves that library: a view carries names and sentences.

use anyhow::{Result, bail};
use hq_bridge::claude::{self, Part, Report, Runner, Targets};
use hq_db::Database;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// One hub and where it stands in Claude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HubView {
    pub name: String,
    /// One sentence each for Claude Code and Claude Desktop.
    pub lines: [String; 2],
    /// Whether either place is set up for this hub.
    pub added: bool,
    pub failed: bool,
}

impl HubView {
    fn new(name: &str, report: &Report) -> Self {
        let parts = [&report.code, &report.desktop];
        Self {
            name: name.into(),
            lines: report.lines(),
            added: parts
                .iter()
                .any(|p| matches!(p, Part::Added(_) | Part::Present(_))),
            failed: parts.iter().any(|p| matches!(p, Part::Failed(_))),
        }
    }
}

/// Claude Desktop starts `hq`, so it needs the program's own absolute path, never a bare name.
pub fn hq_program(program: &Path) -> Result<PathBuf> {
    if program.is_absolute() {
        return Ok(program.to_path_buf());
    }
    bail!("HQ's own program was not found; set its path in Settings, then try again")
}

/// Every paired hub with its state in Claude.
pub fn views(db: &Database, targets: &Targets) -> Result<Vec<HubView>> {
    Ok(claude::hubs(db)?
        .iter()
        .map(|hub| HubView::new(&hub.name, &claude::status(targets, hub)))
        .collect())
}

/// Adds the named hub to Claude, or takes it out.
pub fn change(db: &Database, targets: &Targets, hub: &str, add: bool) -> Result<HubView> {
    let hub = claude::pick(&claude::hubs(db)?, Some(hub))?;
    let report = if add {
        claude::add(targets, &hub)
    } else {
        claude::remove(targets, &hub)
    };
    Ok(HubView::new(&hub.name, &report))
}

/// What a revoke tells the window about Claude, when the cleanup did not fully work. The revoke
/// itself goes ahead either way.
pub fn revoke_warning(cleanup: Result<Option<Report>>) -> Option<String> {
    let report = match cleanup {
        Ok(None) => return None,
        Ok(Some(report)) => report,
        Err(e) => return Some(format!("HQ could not be removed from Claude: {e:#}")),
    };
    let failed = [&report.code, &report.desktop]
        .iter()
        .any(|p| matches!(p, Part::Failed(_)));
    failed.then(|| {
        format!(
            "HQ could not be fully removed from Claude. {}",
            report.lines().join(". ")
        )
    })
}

/// Runs `work` on a blocking thread with this computer's `claude`, since the program is waited for.
pub async fn blocking<T: Send + 'static>(
    db_path: PathBuf,
    hq_bin: PathBuf,
    work: impl FnOnce(&Database, &Targets) -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || {
        let db = Database::open(&db_path)?;
        let runner = claude::SystemRunner::new();
        let targets = Targets {
            runner: &runner as &dyn Runner,
            desktop_config: claude::desktop_config_path(),
            hq_bin,
        };
        work(&db, &targets)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_bridge::claude::Run;
    use hq_db::{bridge as peers, bridge_setup as rows};
    use std::cell::RefCell;

    const KEY: &str = "hqd_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A `claude` that knows no server until one is added, and remembers every call.
    #[derive(Default)]
    struct Fake {
        added: RefCell<bool>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl Runner for Fake {
        fn available(&self) -> bool {
            true
        }

        fn run(&self, args: &[String], _cwd: &Path) -> Result<Run> {
            self.calls.borrow_mut().push(args.to_vec());
            let mut out = Run {
                ok: true,
                stdout: String::new(),
                stderr: String::new(),
            };
            match args[1].as_str() {
                "get" if *self.added.borrow() => {
                    out.stdout = format!(
                        "{}:\n  URL: http://127.0.0.1:47410/mcp\n  Headers:\n    x-api-key: {KEY}\n",
                        args[2]
                    )
                }
                "get" => out.ok = false,
                "add" => *self.added.borrow_mut() = true,
                _ => *self.added.borrow_mut() = false,
            }
            Ok(out)
        }
    }

    fn paired_db() -> Database {
        let db = Database::open_memory().unwrap();
        peers::add_peer(&db, "aa11", "phone", &[], &[]).unwrap();
        let setup = rows::Setup {
            peer_id: "aa11".into(),
            peer_role: rows::ROLE_HUB.into(),
            peer_name: "phone".into(),
            local_port: 47_410,
            peer_offers: true,
            peer_port: None,
            mcp_key: Some(KEY.into()),
            offered: true,
        };
        assert!(rows::record(&db, &setup, None).unwrap());
        db
    }

    #[test]
    fn the_window_lists_a_hub_adds_it_and_removes_it_without_showing_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let targets = Targets {
            runner: &fake,
            desktop_config: Some(dir.path().join("Claude").join("claude_desktop_config.json")),
            hq_bin: PathBuf::from("/opt/example/bin/hq"),
        };
        let db = paired_db();

        let before = views(&db, &targets).unwrap();
        assert_eq!(before.len(), 1);
        assert!(!before[0].added);

        let added = change(&db, &targets, "phone", true).unwrap();
        assert!(added.added && !added.failed);
        assert!(added.lines[0].contains("added as `hq`"));
        assert!(views(&db, &targets).unwrap()[0].added);

        let removed = change(&db, &targets, "phone", false).unwrap();
        assert!(!removed.added);
        assert!(change(&db, &targets, "nope", true).is_err());

        let shown = format!("{before:?}{added:?}{removed:?}");
        assert!(!shown.contains(KEY));
    }

    #[test]
    fn a_revoke_warns_only_when_the_claude_cleanup_failed() {
        assert_eq!(revoke_warning(Ok(None)), None);
        let fine = Report {
            code: Part::Removed("hq".into()),
            desktop: Part::Absent,
        };
        assert_eq!(revoke_warning(Ok(Some(fine))), None);
        let half = Report {
            code: Part::Failed("`claude mcp` exited with an error: busy".into()),
            desktop: Part::Removed("hq".into()),
        };
        let said = revoke_warning(Ok(Some(half))).unwrap();
        assert!(
            said.contains("busy") && said.contains("Claude Desktop: removed"),
            "{said}"
        );
        let broken = revoke_warning(Err(anyhow::anyhow!("no database"))).unwrap();
        assert!(broken.contains("no database"), "{broken}");
    }

    #[test]
    fn claude_desktop_needs_an_absolute_program_path() {
        assert!(hq_program(Path::new("hq")).is_err());
        assert_eq!(
            hq_program(Path::new("/usr/local/bin/hq")).unwrap(),
            PathBuf::from("/usr/local/bin/hq")
        );
    }
}
