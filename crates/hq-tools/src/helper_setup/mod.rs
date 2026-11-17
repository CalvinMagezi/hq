//! First-run helper setup: HQ finds which coding helpers a computer has, installs the free-tier
//! ones the owner wants, starts each sign-in on that computer, and remembers what is ready. One
//! approval covers the whole batch. Design, security model and limits: `docs/HELPER_SETUP.md`.

pub mod catalog;
pub mod runner;
pub mod service;

use std::sync::Arc;

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use serde_json::{Value, json};

use crate::agent_host::{Host, blocking};
use crate::registry::HqTool;
use crate::util::arg_str;
pub use service::HelperReport;

pub const TOOL: &str = "helper_setup";
/// The actions that install software or open a sign-in, which wait for the owner's one yes.
pub const CHANGING_ACTIONS: &[&str] = &["setup", "install", "login"];

/// Whether `action` changes the computer.
pub fn changes_computer(action: &str) -> bool {
    CHANGING_ACTIONS.contains(&action)
}

/// What approving a `helper_setup` call means, in the owner's words, from the catalog and never
/// from the model's own description. `None` for a call that changes nothing.
pub fn describe(args: &Value) -> Option<String> {
    let action = arg_str(args, "action");
    if !changes_computer(&action) {
        return None;
    }
    let ids: Vec<String> = args
        .get("helpers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let helpers = service::pick(&ids).ok()?;
    let host = arg_str(args, "host");
    let on = if host.is_empty() {
        "the connected computer".to_string()
    } else {
        host
    };
    let installs: Vec<String> = helpers
        .iter()
        .filter(|_| action != "login")
        .map(|h| {
            let how = h
                .installs
                .iter()
                .map(|i| i.label)
                .collect::<Vec<_>>()
                .join(" or ");
            format!("{} ({how})", h.title)
        })
        .collect();
    let names = helpers
        .iter()
        .map(|h| h.title)
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = String::from("If approved, HQ will ");
    if installs.is_empty() {
        text.push_str(&format!("open the sign-in for {names} on {on}."));
    } else {
        text.push_str(&format!(
            "download and run these vendors' own installers on {on} for what is not installed yet: {}.",
            installs.join("; ")
        ));
        if action == "setup" {
            text.push_str(" Then it opens each sign-in on that computer.");
        }
    }
    text.push_str(" It will not ask again for each helper. Nothing is installed outside your home folder except through Homebrew if you use it.");
    Some(text)
}

/// The one computer to set up when the owner named none: the only one that answers. More than
/// one is the owner's call, so it errors with the names.
pub fn only_reachable_host() -> Result<Host> {
    let mut up: Vec<Host> = crate::agent_host::all_hosts()?
        .into_iter()
        .filter(|h| h.version().is_ok())
        .collect();
    match up.len() {
        0 => bail!(
            "No computer is connected to HQ right now, so there is nowhere to set helpers up. Pair a computer or start HQ's host on this one."
        ),
        1 => Ok(up.remove(0)),
        _ => bail!(
            "More than one computer is connected ({}). Say which one to set up.",
            up.iter()
                .map(|h| h.name().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Starts `action` for `ids` (empty: every helper in the catalog) on `host` in the background and
/// returns at once: an install and a sign-in take minutes, longer than a tool call may. The
/// result is in `helper_status` and `helper_setup_runs`, and an inbox note says when it ends.
pub fn start(db: Database, host: Host, action: &str, ids: &[String]) -> Result<Vec<&'static str>> {
    if !changes_computer(action) {
        bail!("'{action}' does not change a computer");
    }
    let helpers = service::pick(ids)?;
    let guard = service::HostGuard::take(host.name())?;
    let titles: Vec<&'static str> = helpers.iter().map(|h| h.title).collect();
    let action = action.to_string();
    tokio::spawn(async move {
        let _guard = guard;
        let name = host.name().to_string();
        let job_db = db.clone();
        let result = blocking(move || {
            let mut reports = Vec::new();
            for helper in helpers {
                let one = match action.as_str() {
                    "install" => service::install(host.as_ref(), &job_db, helper),
                    "login" => service::login(host.as_ref(), &job_db, helper),
                    _ => service::setup_one(host.as_ref(), &job_db, helper),
                };
                reports.push(one);
            }
            reports
        })
        .await;
        announce(&db, &name, result);
    });
    Ok(titles)
}

/// One inbox note saying how the setup ended.
fn announce(db: &Database, host: &str, result: Result<Vec<Result<HelperReport>>>) {
    let (title, body) = match result {
        Err(e) => (format!("Helper setup on {host} stopped"), e.to_string()),
        Ok(reports) => {
            let mut ready = Vec::new();
            let mut lines = Vec::new();
            for r in reports {
                match r {
                    Ok(r) => {
                        if r.ready {
                            ready.push(r.title.clone());
                        }
                        lines.push(format!("{}: {}", r.title, r.message));
                    }
                    Err(e) => lines.push(e.to_string()),
                }
            }
            let title = if ready.is_empty() {
                format!("Helper setup on {host} finished, none ready yet")
            } else {
                format!("Ready on {host}: {}", ready.join(", "))
            };
            (title, lines.join("\n"))
        }
    };
    let item = ValueItem::new("helper-setup", ValueKind::Fyi, title, body);
    if let Err(e) = hq_db::value_items::insert(db, &item) {
        tracing::warn!(error = %e, "could not leave the helper setup note");
    }
}

/// Source and key prefix of the inbox item that offers the setup. Approving it starts the setup.
pub const OFFER_SOURCE: &str = "helper-setup-offer";
pub const OFFER_KEY_PREFIX: &str = "helper-setup-offer:";

/// The computer named by an offer item's dedup key.
pub fn offer_host(dedup_key: &str) -> Option<&str> {
    dedup_key
        .strip_prefix(OFFER_KEY_PREFIX)
        .filter(|h| !h.is_empty())
}

/// Looks at which helpers `host` has and records it, without asking any of them anything.
pub fn refresh_installed(
    host: &dyn crate::agent_host::HostBackend,
    db: &Database,
) -> Result<Vec<HelperReport>> {
    let found = service::detect(host, &catalog::all_programs())?;
    let mut out = Vec::new();
    for helper in catalog::CATALOG {
        let installed = helper.programs.iter().any(|p| found.contains(*p));
        hq_db::helper_status::record(db, host.name(), helper.id, installed, None, None)?;
        out.push(service::report(db, host.name(), helper)?);
    }
    Ok(out)
}

/// The first-run offer: when exactly one computer is connected and none of the catalog's helpers
/// is on it, one inbox item that sets them up when approved. `None` when there is nothing to
/// offer, the owner already has a helper, or the offer is already in the inbox (the dedup key
/// holds it, answered or not, so a dismissed offer does not come back every half hour).
pub fn offer(db: &Database) -> Result<Option<ValueItem>> {
    if hq_db::helper_status::any_installed(db)? {
        return Ok(None);
    }
    let host = match only_reachable_host() {
        Ok(host) => host,
        Err(_) => return Ok(None),
    };
    let reports = refresh_installed(host.as_ref(), db)?;
    if reports.iter().any(|r| r.installed) {
        return Ok(None);
    }
    let names = catalog::CATALOG
        .iter()
        .map(|h| h.title)
        .collect::<Vec<_>>()
        .join(", ");
    let key = format!("{OFFER_KEY_PREFIX}{}", host.name());
    if already_offered(db, &key)? {
        return Ok(None);
    }
    let item = ValueItem::new(
        OFFER_SOURCE,
        ValueKind::ActionNeeded,
        format!("Set up coding helpers on {}", host.name()),
        format!(
            "HQ can set up {names} on {}. They are coding helpers you can start from HQ, and each can be tried with a free account. HQ installs them with each vendor's own installer and opens their sign-in pages on that computer; you only sign in. Approve to start. You will not be asked again for each one.",
            host.name()
        ),
    )
    .with_dedup_key(key)
    .owner_only(true);
    Ok(Some(item))
}

/// Whether an offer with this key was ever left in the inbox, answered, dismissed or expired:
/// the owner said no once, and a half-hourly task must not ask again.
fn already_offered(db: &Database, key: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM value_items WHERE dedup_key = ?1)",
            [key],
            |r| r.get(0),
        )?)
    })
}

pub struct HelperSetupTool {
    db: Arc<Database>,
}

impl HelperSetupTool {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn status(&self) -> Result<Value> {
        let known = service::known(&self.db)?;
        let runs = hq_db::helper_status::latest_runs(&self.db)?;
        let hosts: Vec<Value> = crate::agent_host::native_host_names()
            .into_iter()
            .map(|name| {
                let rows = known.get(&name).cloned().unwrap_or_default();
                json!({ "host": name, "checked": !rows.is_empty(), "helpers": rows,
                        "runs": runs.iter().filter(|r| r.host == name).collect::<Vec<_>>() })
            })
            .collect();
        let catalog: Vec<Value> = catalog::CATALOG
            .iter()
            .map(|h| json!({ "id": h.id, "title": h.title, "about": h.blurb, "free": h.free_note }))
            .collect();
        Ok(json!({
            "hosts": hosts,
            "catalog": catalog,
            "next": "A helper is ready when it is installed and signed in. `setup` installs what is missing and opens each sign-in on that computer; the owner approves it once. HQ does the rest, and the owner only signs in on each vendor's own page.",
        }))
    }
}

#[async_trait]
impl HqTool for HelperSetupTool {
    fn name(&self) -> &str {
        TOOL
    }
    fn description(&self) -> &str {
        "Coding helpers on the owner's computer(s): which are installed and signed in, and setting up the free ones (Cursor, Codex, Antigravity). `status` reads what HQ last saw (no changes, no approval). `check` looks again at one computer now. `setup` installs what is missing and starts each sign-in on that computer; `install` and `login` do one half. These change the computer, so the owner approves the call once, and nothing else is asked per helper. The commands are fixed by HQ; you only choose the computer (`host`, optional when one is connected) and which helpers (`helpers`, default all). It returns at once and works in the background: the owner sees sign-in pages open on that computer and gets an inbox note when it ends. Offer this when no helper is ready, when a helper is not installed, or when a job needs one."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["status", "check", "setup", "install", "login"], "default": "status" },
                "host": { "type": "string", "description": "The computer; leave out when only one is connected" },
                "helpers": { "type": "array", "items": { "type": "string", "enum": catalog::CATALOG.iter().map(|h| h.id).collect::<Vec<_>>() },
                             "description": "Which helpers; leave out for all" }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let action = match arg_str(&args, "action").as_str() {
            "" => "status".to_string(),
            other => other.to_string(),
        };
        if action == "status" {
            return self.status();
        }
        let ids: Vec<String> = args
            .get("helpers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let named = arg_str(&args, "host");
        let host = if named.is_empty() {
            blocking(only_reachable_host).await??
        } else {
            let name = named.clone();
            blocking(move || crate::agent_host::host(Some(&name))).await??
        };
        match action.as_str() {
            "check" => {
                let db = self.db.clone();
                let helpers = service::pick(&ids)?;
                let name = host.name().to_string();
                let reports = blocking(move || {
                    helpers
                        .into_iter()
                        .map(|h| service::check(host.as_ref(), &db, h))
                        .collect::<Result<Vec<_>>>()
                })
                .await??;
                Ok(json!({ "host": name, "helpers": reports }))
            }
            a if changes_computer(a) => {
                let name = host.name().to_string();
                let titles = start((*self.db).clone(), host, a, &ids)?;
                Ok(json!({
                    "started": true, "host": name, "helpers": titles,
                    "tell_the_owner": format!("Setting up {} on {name}. Pages to sign in will open on that computer; the owner only has to sign in on each one. This takes a few minutes. Look at `helper_setup` status to follow it.", titles.join(", ")),
                }))
            }
            other => bail!("unknown action '{other}'"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offer_names_its_computer_in_its_key() {
        assert_eq!(offer_host("helper-setup-offer:laptop"), Some("laptop"));
        assert_eq!(offer_host("helper-setup-offer:"), None);
        assert_eq!(offer_host("owner-approval:web:t1:bash:abc"), None);
    }

    #[test]
    fn a_dismissed_offer_is_never_made_again() {
        let db = Database::open_memory().unwrap();
        let key = format!("{OFFER_KEY_PREFIX}laptop");
        assert!(!already_offered(&db, &key).unwrap());
        let item = ValueItem::new(OFFER_SOURCE, ValueKind::ActionNeeded, "t", "b")
            .with_dedup_key(key.clone());
        hq_db::value_items::insert(&db, &item).unwrap();
        hq_db::value_items::dismiss_by_id(&db, &item.id).unwrap();
        assert!(already_offered(&db, &key).unwrap());
    }

    #[test]
    fn the_approval_text_comes_from_the_catalog_and_only_for_changing_actions() {
        let all = describe(&json!({"action": "setup"})).unwrap();
        for h in catalog::CATALOG {
            assert!(all.contains(h.title), "{all}");
        }
        let login = describe(&json!({"action": "login", "helpers": ["codex"]})).unwrap();
        assert!(
            login.contains("sign-in") && !login.contains("installers"),
            "{login}"
        );
        assert!(describe(&json!({"action": "status"})).is_none());
        assert!(describe(&json!({"action": "setup", "helpers": ["nope"]})).is_none());
    }
}
