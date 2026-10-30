//! A bounded caller (a claude.ai connector, or the handoff key while `handoff_cwd_allow` lists
//! anything) asking for a folder it may not use files a request with the owner instead of hitting
//! a dead end. The owner's signed approval (chat button, web inbox, `hq queue approve`) becomes a
//! grant for that folder and everything under it on that host, kept until revoked
//! (`hq_db::folder_grants`). Nothing an MCP caller can reach signs an approval or writes a grant.

use super::*;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::{folder_grants, value_items};
use value_items::RequestState;

const SOURCE: &str = "session-folders";
const KEY_PREFIX: &str = "folder-grant:";
/// Requests one caller may have waiting at once, so a hijacked conversation cannot flood the owner.
const MAX_WAITING_PER_CALLER: i64 = 5;
/// How long a request, and a decline, lasts.
const REQUEST_TTL_HOURS: i64 = 24;
/// Who a handoff-key request is from; that key carries no name of its own.
pub(super) const HANDOFF_REQUESTER: &str = "handoff key";

/// Longest folder a request may name: real project paths are far shorter, and the owner must be
/// able to read all of it in a notification.
const MAX_PATH_CHARS: usize = 300;
/// Between the parts of a request key; labels and host names never contain it.
const KEY_SEP: char = '\u{1f}';

/// One spelling per folder, so `/a/b/`, `/a//b` and `/a/./b` are one request and one grant.
fn normalized(path: &Path) -> PathBuf {
    path.components().collect()
}

fn request_key(requester: &str, host: &str, path: &Path) -> String {
    format!(
        "{KEY_PREFIX}{requester}{KEY_SEP}{host}{KEY_SEP}{}",
        normalized(path).display()
    )
}

fn requester_prefix(requester: &str) -> String {
    format!("{KEY_PREFIX}{requester}{KEY_SEP}")
}

/// The bound's folders plus the owner's grants on `host`. An approved request for `wanted` is
/// spent into a grant first, so the call that follows the owner's approval goes through.
pub(super) fn granted_folders(
    db: &Database,
    host: &str,
    requester: &str,
    wanted: Option<&str>,
) -> Result<Vec<String>> {
    if let Some(path) = wanted.map(str::trim).filter(|p| !p.is_empty()) {
        let key = request_key(requester, host, Path::new(path));
        // The grant is written before the approval is spent, so a failed write loses nothing and
        // two calls racing on one approval both find the grant.
        if value_items::is_signed_approval(db, &key)? && approval_unspent(db, &key)? {
            let folder = normalized(Path::new(path));
            folder_grants::grant(db, host, &folder.to_string_lossy(), requester)?;
            value_items::consume_approval(db, &key)?;
        }
    }
    folder_grants::folders_for_host(db, host)
}

fn approval_unspent(db: &Database, key: &str) -> Result<bool> {
    let latest = value_items::latest_engagement_by_dedup(db, key)?;
    let now = chrono::Utc::now();
    Ok(latest.is_some_and(|l| {
        l.engagement.as_deref() == Some("approved") && l.expires_at.is_none_or(|at| at > now)
    }))
}

/// The error a refused folder returns: a request filed (or already waiting, or declined) and what
/// the caller should do next. Never `Ok`: the session does not start on this call.
pub(super) fn ask_owner(db: &Database, host: &str, requester: &str, path: &Path) -> anyhow::Error {
    let path = &normalized(path);
    let raw = path.to_string_lossy();
    if raw.chars().count() > MAX_PATH_CHARS || hq_db::tasks::has_hidden_chars(&raw) {
        return anyhow::anyhow!(
            "cwd '{}' is too long or has control or invisible characters, so it cannot be shown to the \
             owner for approval; no session was started",
            raw.escape_debug()
        );
    }
    let key = request_key(requester, host, path);
    let shown = path.display();
    let latest = match value_items::latest_engagement_by_dedup(db, &key) {
        Ok(latest) => latest,
        Err(_) => return lookup_failed(),
    };
    match RequestState::from_latest(latest, chrono::Utc::now()) {
        RequestState::Declined => anyhow::anyhow!(
            "The owner declined coding sessions in {shown} on host '{host}'. Do not ask for this folder \
             again or start the session somewhere else to reach it; ask the owner what they want instead."
        ),
        RequestState::Wait => pending(&shown, host),
        RequestState::Ask => match file_request(db, &key, host, requester, path) {
            Ok(()) => pending(&shown, host),
            Err(e) => e,
        },
    }
}

fn file_request(
    db: &Database,
    key: &str,
    host: &str,
    requester: &str,
    path: &Path,
) -> Result<(), anyhow::Error> {
    let prefix = requester_prefix(requester);
    let waiting =
        value_items::count_waiting_with_prefix(db, &prefix).map_err(|_| lookup_failed())?;
    if waiting >= MAX_WAITING_PER_CALLER {
        return Err(anyhow::anyhow!(
            "{MAX_WAITING_PER_CALLER} folder requests from this caller are already waiting for the owner. \
             Wait for them to answer before asking for another folder."
        ));
    }
    let shown = path.display();
    let mut item = ValueItem::new(
        SOURCE,
        ValueKind::ActionNeeded,
        format!("Allow coding sessions in {shown}?"),
        format!(
            "\"{requester}\" asks to start coding sessions in {shown} on host '{host}'. Approving lets \
             every Claude connection and the handoff key start sessions in that folder and everything \
             under it on that host, until you revoke it. Dismiss to refuse. If you did not ask for \
             this, dismiss it."
        ),
    )
    .with_dedup_key(key.to_string());
    item.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(REQUEST_TTL_HOURS));
    value_items::insert(db, &item).map_err(|_| lookup_failed())?;
    Ok(())
}

fn pending(shown: &std::path::Display<'_>, host: &str) -> anyhow::Error {
    anyhow::Error::new(AwaitingApproval {
        folder: shown.to_string(),
        host: host.to_string(),
        waited: None,
    })
}

/// A folder request that is filed and waiting on the owner. Always an error, so no caller can
/// read it as a session that started.
#[derive(Debug)]
pub(super) struct AwaitingApproval {
    folder: String,
    host: String,
    /// How long this call waited for the answer, once it has.
    waited: Option<Duration>,
}

impl std::fmt::Display for AwaitingApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { folder, host, .. } = self;
        let asked = match self.waited {
            Some(w) => format!(
                "HQ asked the owner to approve it and waited {}s without an answer",
                w.as_secs()
            ),
            None => "HQ asked the owner to approve it".to_string(),
        };
        write!(
            f,
            "BLOCKED: no session was started. {folder} on host '{host}' is not yet a folder sessions may \
             start in; {asked} (their chat, the web inbox and a push notification). Tell the user you are \
             waiting on this approval, then call this tool again with exactly the same cwd and arguments: \
             each call waits for the answer and starts the session as soon as they approve. Do not report a \
             session as started, do not start it in another folder and cd into this one, and do not try to \
             change agent_host settings."
        )
    }
}

impl std::error::Error for AwaitingApproval {}

/// What the owner has said so far about a folder request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Answer {
    Approved,
    Declined,
    Waiting,
}

fn answer(db: &Database, host: &str, requester: &str, path: &Path) -> Result<Answer> {
    let key = request_key(requester, host, path);
    if value_items::is_signed_approval(db, &key)? && approval_unspent(db, &key)? {
        return Ok(Answer::Approved);
    }
    let latest = value_items::latest_engagement_by_dedup(db, &key)?;
    Ok(
        match RequestState::from_latest(latest, chrono::Utc::now()) {
            RequestState::Declined => Answer::Declined,
            RequestState::Wait | RequestState::Ask => Answer::Waiting,
        },
    )
}

/// How often a waiting call looks for the owner's answer.
const ANSWER_POLL: Duration = Duration::from_millis(500);
/// An MCP client gives up on a call near 60 seconds, and the launch that follows an approval
/// takes up to the launch bound, so the wait is what is left of 55 seconds, at most this.
const MAX_ANSWER_WAIT: Duration = Duration::from_secs(30);

/// How long a spawn waits for the owner to answer a folder request it just filed.
pub(super) fn answer_wait(cfg: &hq_core::config::AgentHostConfig) -> Duration {
    let launch = cfg.launch_bound_secs.clamp(
        hq_core::config::MIN_LAUNCH_BOUND_SECS,
        hq_core::config::MAX_LAUNCH_BOUND_SECS,
    );
    Duration::from_secs(55u64.saturating_sub(launch)).min(MAX_ANSWER_WAIT)
}

/// Waits up to `budget` for the owner to approve or decline the request for `path`. An answer
/// that cannot be read is `Waiting`: the caller reports the block and nothing starts.
pub(super) async fn wait_for_answer(
    db: &Database,
    host: &str,
    requester: &str,
    path: &Path,
    budget: Duration,
) -> Answer {
    let path = normalized(path);
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        match answer(db, host, requester, &path) {
            Ok(Answer::Waiting) | Err(_) => {}
            Ok(decided) => return decided,
        }
        if tokio::time::Instant::now() >= deadline {
            return Answer::Waiting;
        }
        tokio::time::sleep(ANSWER_POLL.min(deadline - tokio::time::Instant::now())).await;
    }
}

/// The block reported after a call waited `waited` and the owner has not answered.
pub(super) fn still_waiting(path: &Path, host: &str, waited: Duration) -> anyhow::Error {
    anyhow::Error::new(AwaitingApproval {
        folder: normalized(path).display().to_string(),
        host: host.to_string(),
        waited: Some(waited),
    })
}

fn lookup_failed() -> anyhow::Error {
    anyhow::anyhow!(
        "HQ could not check the owner's folder approvals just now; nothing started. Try again shortly."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        (dir, db)
    }

    fn requests(db: &Database, key: &str) -> Vec<String> {
        db.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id FROM value_items WHERE dedup_key = ?1")?;
            let rows = stmt.query_map([key], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .unwrap()
    }

    #[test]
    fn a_refused_folder_asks_once_and_the_owners_approval_becomes_a_lasting_grant() {
        let (_dir, db) = setup();
        let path = Path::new("/srv/work/app");
        let key = request_key("claude", "laptop", path);
        let first = ask_owner(&db, "laptop", "claude", path).to_string();
        assert!(first.contains("asked the owner"), "{first}");
        assert!(first.contains("same cwd"), "{first}");
        let again = ask_owner(&db, "laptop", "claude", path).to_string();
        assert!(again.contains("asked the owner"), "{again}");
        let filed = requests(&db, &key);
        assert_eq!(filed.len(), 1, "a waiting request is not filed twice");

        let wanted = Some("/srv/work/app");
        assert!(
            granted_folders(&db, "laptop", "claude", wanted)
                .unwrap()
                .is_empty()
        );
        assert!(value_items::approve_by_id(&db, &filed[0]).unwrap());
        assert_eq!(
            granted_folders(&db, "laptop", "claude", wanted).unwrap(),
            vec!["/srv/work/app"]
        );
        assert_eq!(
            granted_folders(&db, "laptop", "claude", None).unwrap(),
            vec!["/srv/work/app"],
            "the grant outlives the approval it was made from"
        );
        assert!(
            granted_folders(&db, "server", "claude", None)
                .unwrap()
                .is_empty()
        );

        let cfg = hq_core::config::AgentHostConfig {
            handoff_cwd_allow: granted_folders(&db, "laptop", "claude", None).unwrap(),
            ..Default::default()
        };
        assert!(require_cwd_in(Some("/srv/work/app/sub"), &cfg, CwdBound::Connector).is_ok());
        let outside =
            require_cwd_in(Some("/srv/work/other"), &cfg, CwdBound::Connector).unwrap_err();
        assert!(outside.downcast_ref::<OutsideAllowedFolders>().is_some());
    }

    #[tokio::test]
    async fn a_waiting_call_sees_an_approval_that_lands_while_it_waits() {
        let (_dir, db) = setup();
        let db = Arc::new(db);
        let path = Path::new("/srv/work/quick");
        let asked = ask_owner(&db, "laptop", "claude", path);
        assert!(
            asked.downcast_ref::<AwaitingApproval>().is_some(),
            "{asked}"
        );
        let id = requests(&db, &request_key("claude", "laptop", path)).remove(0);
        let owner = db.clone();
        let approving = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            assert!(value_items::approve_by_id(&owner, &id).unwrap());
        });
        let answer = wait_for_answer(&db, "laptop", "claude", path, Duration::from_secs(10)).await;
        approving.join().unwrap();
        assert_eq!(answer, Answer::Approved);
    }

    #[tokio::test]
    async fn no_answer_is_a_block_that_says_nothing_started_and_a_decline_ends_the_wait() {
        let (_dir, db) = setup();
        let path = Path::new("/srv/work/slow");
        ask_owner(&db, "laptop", "claude", path);
        let budget = Duration::from_millis(200);
        let answer = wait_for_answer(&db, "laptop", "claude", path, budget).await;
        assert_eq!(answer, Answer::Waiting);
        let blocked = still_waiting(path, "laptop", budget).to_string();
        assert!(
            blocked.starts_with("BLOCKED: no session was started"),
            "{blocked}"
        );
        assert!(blocked.contains("same cwd"), "{blocked}");

        let id = requests(&db, &request_key("claude", "laptop", path)).remove(0);
        assert!(value_items::dismiss_by_id(&db, &id).unwrap());
        let started = std::time::Instant::now();
        let answer = wait_for_answer(&db, "laptop", "claude", path, Duration::from_secs(10)).await;
        assert_eq!(answer, Answer::Declined);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a decline does not wait out the budget"
        );
    }

    #[test]
    fn the_wait_leaves_room_for_the_launch_inside_a_minute() {
        let cfg = |secs| hq_core::config::AgentHostConfig {
            launch_bound_secs: secs,
            ..Default::default()
        };
        assert_eq!(answer_wait(&cfg(25)), Duration::from_secs(30));
        assert_eq!(answer_wait(&cfg(50)), Duration::from_secs(5));
        for secs in [0, 5, 25, 50, 500] {
            let launch = secs.clamp(
                hq_core::config::MIN_LAUNCH_BOUND_SECS,
                hq_core::config::MAX_LAUNCH_BOUND_SECS,
            );
            assert!(answer_wait(&cfg(secs)).as_secs() + launch < 60);
        }
    }

    #[test]
    fn a_declined_folder_is_not_asked_again_and_a_flood_is_capped() {
        let (_dir, db) = setup();
        let path = Path::new("/srv/secret");
        ask_owner(&db, "laptop", "claude", path);
        let id = requests(&db, &request_key("claude", "laptop", path)).remove(0);
        assert!(value_items::dismiss_by_id(&db, &id).unwrap());
        let declined = ask_owner(&db, "laptop", "claude", path).to_string();
        assert!(declined.contains("declined"), "{declined}");
        assert_eq!(
            requests(&db, &request_key("claude", "laptop", path)).len(),
            1
        );

        for n in 0..MAX_WAITING_PER_CALLER {
            ask_owner(&db, "laptop", "claude", Path::new(&format!("/srv/p{n}")));
        }
        let capped = ask_owner(&db, "laptop", "claude", Path::new("/srv/one-more"));
        assert!(capped.to_string().contains("already waiting"), "{capped}");
        let other = ask_owner(&db, "laptop", "other", Path::new("/srv/one-more"));
        assert!(
            other.to_string().contains("asked the owner"),
            "the cap is per caller"
        );
    }

    /// Approving and granting are owner actions. No code an MCP caller can reach may sign an
    /// approval, and only this module writes a grant (after spending a signed approval).
    #[test]
    fn nothing_reachable_over_mcp_approves_or_grants() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut offenders = Vec::new();
        for crate_dir in ["hq-tools/src", "hq-mcp/src", "hq-agent/src"] {
            scan(&root.join(crate_dir), &mut offenders);
        }
        assert!(
            offenders.is_empty(),
            "owner-only calls in agent-reachable code: {offenders:?}"
        );
    }

    fn scan(dir: &Path, offenders: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan(&path, offenders);
                continue;
            }
            let name = path.to_string_lossy();
            let is_test = name.ends_with("tests.rs") || name.contains("/tests/");
            if !name.ends_with(".rs") || is_test {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let body = text.split("#[cfg(test)]").next().unwrap_or_default();
            let grants_here = name.ends_with("folder_request.rs");
            let bad = ["approve_by_id(", "record_engagement_by_token("]
                .iter()
                .any(|c| body.contains(c))
                || (!grants_here && body.contains("folder_grants::grant("));
            if bad {
                offenders.push(name.into_owned());
            }
        }
    }
}

#[cfg(test)]
mod spelling_tests {
    use super::*;

    #[test]
    fn spellings_of_one_folder_share_a_request_and_hidden_text_is_refused() {
        let key = |p: &str| request_key("claude", "laptop", Path::new(p));
        assert_eq!(key("/srv/app"), key("/srv/app/"));
        assert_eq!(key("/srv/app"), key("/srv//app"));
        assert_eq!(key("/srv/app"), key("/srv/./app"));
        assert!(!key("/srv/app").starts_with(&requester_prefix("claude:x")));
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        let spoof = ask_owner(&db, "laptop", "claude", Path::new("/srv/a\u{202e}b"));
        assert!(spoof.to_string().contains("invisible"), "{spoof}");
        let long = format!("/srv/{}", "a".repeat(MAX_PATH_CHARS));
        assert!(
            ask_owner(&db, "laptop", "claude", Path::new(&long))
                .to_string()
                .contains("too long")
        );
    }
}
