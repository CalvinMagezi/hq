//! What the owner's answer to a sandbox access request can leave behind: standing access for the
//! task or the repository, or a starter set of common toolchain caches and registries. Called only
//! from the owner's own routes. A helper's request names the resource; the owner's choice names
//! the scope, and nothing a helper writes widens either.

use super::sandbox_request::HostPaths;
use anyhow::{Result, bail};
use hq_core::grants::{
    Capability, GrantRequest, STARTER_PROFILES, normalize, starter_for, starter_profile,
};
use hq_db::Database;
use hq_db::sandbox_grants::{self as store, Grant};

const KEY_PREFIX: &str = "sandbox-grant:";
const SOURCE: &str = "sandbox-grants";
/// How long a standing rule lasts unless the owner revokes it first.
const RULE_TTL_SECS: i64 = 90 * 24 * 3600;

/// The inbox answers that widen access beyond the one request.
pub const ALLOW_TASK: &str = "allow_task";
pub const ALLOW_REPO: &str = "allow_repo";
pub const ALLOW_STARTER: &str = "allow_starter";

/// Whether `action` is one of this module's answers.
pub fn is_access_action(action: &str) -> bool {
    matches!(action, ALLOW_TASK | ALLOW_REPO | ALLOW_STARTER)
}

/// The access request an inbox item stands for, when it is one: its dedup key names the grant.
pub fn grant_of_item(db: &Database, item_id: &str) -> Option<Grant> {
    // The item must be HQ's own: its source is checked as well as its key, so a notification a
    // helper wrote under a guessed key is never taken for an access request.
    let (key, source): (Option<String>, Option<String>) = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT dedup_key, source_task FROM value_items WHERE id = ?1",
                [item_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .ok()?;
    if source.as_deref() != Some(SOURCE) {
        return None;
    }
    let id = key?.strip_prefix(KEY_PREFIX)?.to_string();
    store::get(db, &id).ok().flatten()
}

/// What an answer would add: the scope, each `(capability, resource)` rule, and the note kept with it.
struct Plan {
    scope: String,
    items: Vec<(Capability, String)>,
    note: String,
}

/// The rules an answer would add, or why it cannot: nothing is written. `starter` is the name the
/// owner chose, for `allow_starter`.
fn plan(grant: &Grant, action: &str, starter: Option<&str>) -> Result<Plan> {
    if grant.capability == store::PROFILE || grant.scope != "session" {
        bail!("that is not a helper's access request");
    }
    // A request the owner revoked, or that lapsed, cannot leave a permanent rule behind.
    if grant.state != store::PENDING && grant.state != store::ACTIVE {
        bail!("that request is no longer open");
    }
    let scope = match action {
        ALLOW_TASK => match grant.task_id.as_deref() {
            Some(t) if !t.is_empty() => format!("{}{t}", store::TASK_SCOPE),
            _ => bail!("this request is not on a task, so it cannot be allowed for one"),
        },
        ALLOW_REPO | ALLOW_STARTER => {
            if grant.repo.trim().is_empty() {
                bail!("HQ does not know this request's repository");
            }
            format!("{}{}", store::REPO_SCOPE, grant.repo)
        }
        _ => bail!("not an access answer"),
    };
    if action == ALLOW_STARTER {
        let name = starter.unwrap_or_default();
        if !STARTER_PROFILES.contains(&name) {
            bail!("unknown starter '{name}'");
        }
        let p = starter_profile(name).ok_or_else(|| anyhow::anyhow!("unknown starter"))?;
        let mut items = Vec::new();
        items.extend(p.writable.iter().map(|x| (Capability::FsWrite, x.clone())));
        items.extend(p.readable.iter().map(|x| (Capability::FsRead, x.clone())));
        items.extend(
            p.loopback_ports
                .iter()
                .map(|x| (Capability::Loopback, x.to_string())),
        );
        items.extend(p.egress.iter().map(|x| {
            let r = normalize(&GrantRequest {
                capability: Capability::Egress,
                resource: x.clone(),
            });
            (Capability::Egress, r.resource)
        }));
        return Ok(Plan {
            scope,
            items,
            note: format!("starter {name}"),
        });
    }
    let cap: Capability = serde_json::from_value(serde_json::json!(grant.capability))
        .map_err(|_| anyhow::anyhow!("unknown capability"))?;
    let what = if action == ALLOW_TASK {
        "allowed for this task"
    } else {
        "allowed for this repository"
    };
    Ok(Plan {
        scope,
        items: vec![(cap, grant.resource.clone())],
        note: what.to_string(),
    })
}

/// Refuses an answer that cannot be applied, before anything is recorded.
pub fn check_answer(
    db: &Database,
    item_id: &str,
    action: &str,
    starter: Option<&str>,
) -> Result<()> {
    let Some(grant) = grant_of_item(db, item_id) else {
        bail!("that item is not a helper's access request");
    };
    plan(&grant, action, starter).map(|_| ())
}

/// Adds the standing access an `allow_*` answer asked for. Returns how many rules are new.
pub fn apply_answer(
    db: &Database,
    item_id: &str,
    action: &str,
    starter: Option<&str>,
    now: i64,
) -> Result<usize> {
    let Some(grant) = grant_of_item(db, item_id) else {
        bail!("that item is not a helper's access request");
    };
    let Plan { scope, items, note } = plan(&grant, action, starter)?;
    let mut added = 0;
    for (cap, resource) in items {
        let capability = serde_json::to_value(cap)?
            .as_str()
            .unwrap_or_default()
            .to_string();
        // The rule carries no delta of its own: what it opens is worked out against the session
        // computer's home each time a session starts or asks.
        if store::add_rule(
            db,
            &scope,
            &capability,
            &resource,
            &note,
            now,
            Some(now + RULE_TTL_SECS),
        )?
        .is_some()
        {
            added += 1;
        }
    }
    Ok(added)
}

/// The starter set a pending request falls inside, so the first card can offer it. Reads the home
/// of the computer the session runs on; `None` when that computer cannot say.
pub fn suggested_starter(grant: &Grant) -> Option<&'static str> {
    if grant.state != store::PENDING {
        return None;
    }
    let cap: Capability = serde_json::from_value(serde_json::json!(grant.capability)).ok()?;
    let host = crate::agent_host::host(Some(&grant.host)).ok()?;
    let paths = HostPaths::probe(host)?;
    let req = GrantRequest {
        capability: cap,
        resource: grant.resource.clone(),
    };
    starter_for(&req, &hq_core::grants::PathView::home(&paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::grants::SandboxDelta;
    use hq_core::types::{ValueItem, ValueKind};

    fn pending(
        db: &Database,
        task: Option<&str>,
        repo: &str,
        cap: &str,
        resource: &str,
    ) -> (String, String) {
        let d = SandboxDelta::default();
        let id = store::file(
            db,
            &store::NewGrant {
                session_id: "s1",
                task_id: task,
                host: "local",
                repo,
                capability: cap,
                resource,
                delta: &d,
                state: store::PENDING,
                decided_by: store::BY_SYSTEM,
                helper_reason: "",
                decision_note: "",
                expires_at: None,
            },
            1,
        )
        .unwrap();
        let item = ValueItem::new("sandbox-grants", ValueKind::ActionNeeded, "t", "b")
            .with_dedup_key(format!("{KEY_PREFIX}{id}"));
        let item_id = item.id.clone();
        hq_db::value_items::insert(db, &item).unwrap();
        (id, item_id)
    }

    #[test]
    fn allow_task_and_allow_repo_add_one_rule_for_the_resource_asked() {
        let db = Database::open_memory().unwrap();
        let (_, item) = pending(&db, Some("t1"), "/repo", "fs_write", "/repo/cache");
        assert_eq!(apply_answer(&db, &item, ALLOW_TASK, None, 5).unwrap(), 1);
        assert_eq!(
            apply_answer(&db, &item, ALLOW_TASK, None, 6).unwrap(),
            0,
            "idempotent"
        );
        assert_eq!(apply_answer(&db, &item, ALLOW_REPO, None, 7).unwrap(), 1);
        let rules = store::rules_for(&db, &["task:t1".into(), "repo:/repo".into()], 10).unwrap();
        assert_eq!(rules.len(), 2);
        assert!(
            rules
                .iter()
                .all(|r| r.resource == "/repo/cache" && r.capability == "fs_write")
        );
    }

    #[test]
    fn a_task_answer_needs_a_task_and_a_starter_must_be_one_of_ours() {
        let db = Database::open_memory().unwrap();
        let (_, item) = pending(&db, None, "/repo", "egress", "pub.dev:443");
        assert!(check_answer(&db, &item, ALLOW_TASK, None).is_err());
        assert!(check_answer(&db, &item, ALLOW_STARTER, Some("everything")).is_err());
        assert!(check_answer(&db, &item, ALLOW_STARTER, None).is_err());
        assert!(check_answer(&db, &item, ALLOW_REPO, None).is_ok());
        assert!(check_answer(&db, "no-such-item", ALLOW_REPO, None).is_err());
    }

    #[test]
    fn a_starter_adds_its_whole_set_for_the_repository_once() {
        let db = Database::open_memory().unwrap();
        let (_, item) = pending(&db, Some("t1"), "/repo", "egress", "pub.dev:443");
        let n = apply_answer(&db, &item, ALLOW_STARTER, Some("mobile-dev"), 5).unwrap();
        let p = starter_profile("mobile-dev").unwrap();
        assert_eq!(
            n,
            p.writable.len() + p.readable.len() + p.loopback_ports.len() + p.egress.len()
        );
        assert_eq!(
            apply_answer(&db, &item, ALLOW_STARTER, Some("mobile-dev"), 6).unwrap(),
            0
        );
        let rules = store::rules_for(&db, &["repo:/repo".into()], 10).unwrap();
        assert!(
            rules
                .iter()
                .any(|r| r.capability == "egress" && r.resource == "pub.dev:443")
        );
        assert!(rules.iter().any(|r| r.resource == "~/.gradle/caches"));
        assert!(
            rules.iter().all(|r| r.expires_at.is_some()),
            "rules lapse unless renewed"
        );
    }

    #[test]
    fn profile_bindings_and_rules_are_not_access_requests() {
        let db = Database::open_memory().unwrap();
        let g = store::bind_profile(&db, "s1", "mobile", 1).unwrap();
        let grant = store::get(&db, &g).unwrap().unwrap();
        assert!(plan(&grant, ALLOW_REPO, None).is_err());
    }

    #[test]
    fn a_notification_a_helper_wrote_under_a_guessed_key_is_not_an_access_request() {
        let db = Database::open_memory().unwrap();
        let (id, _) = pending(&db, Some("t1"), "/repo", "fs_write", "/repo/x");
        let decoy = ValueItem::new("notify_owner", ValueKind::Insight, "Looks official", "body")
            .with_dedup_key(format!("{KEY_PREFIX}{id}"));
        let decoy_id = decoy.id.clone();
        hq_db::value_items::insert(&db, &decoy).unwrap();
        assert!(grant_of_item(&db, &decoy_id).is_none());
        assert!(check_answer(&db, &decoy_id, ALLOW_REPO, None).is_err());
        assert!(hq_core::grants::STARTER_PROFILES.len() == 3);
    }

    #[test]
    fn a_revoked_or_lapsed_request_cannot_leave_a_permanent_rule() {
        let db = Database::open_memory().unwrap();
        let (id, item) = pending(&db, Some("t1"), "/repo", "fs_write", "/repo/x");
        assert!(check_answer(&db, &item, ALLOW_REPO, None).is_ok());
        store::revoke(&db, &id, 5).unwrap();
        assert!(check_answer(&db, &item, ALLOW_REPO, None).is_err());
        assert!(apply_answer(&db, &item, ALLOW_REPO, None, 6).is_err());
    }
}
