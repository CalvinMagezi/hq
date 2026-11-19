use super::*;
use crate::harness_session::{CALLER_SESSION_ARG, CONNECTOR_ARG};
use hq_db::harness_sessions_registry::{self as sessions, NewSession, Placement};
use hq_db::provenance::ReviewState;
use hq_db::tasks::{self as t, LeaseIdentity};
use serde_json::json;

struct Fixture {
    db: Arc<Database>,
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            // The migrated schema seeds the "personal" space.
            t::create_initiative(c, "in-1", "personal", None, "Work", "work", "WORK")?;
            Ok(())
        })
        .unwrap();
        Self {
            db,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn recorder(&self) -> Recorder {
        Recorder::new(self.db.clone(), self.dir.path().to_path_buf())
    }

    fn task(&self, id: &str) {
        self.db
            .with_conn(|c| {
                t::create_task(
                    c,
                    id,
                    "in-1",
                    &t::NewTask {
                        title: "A task",
                        created_by: "test",
                        ..Default::default()
                    },
                )
                .map(|_| ())
            })
            .unwrap();
    }

    /// A native lease on `task`, returning the lease id (not the token).
    fn lease(&self, task: &str, actor: &str) -> String {
        self.db
            .with_conn(|c| {
                let who = LeaseIdentity {
                    actor,
                    harness: "native",
                    external_session_ref: actor,
                    ..Default::default()
                };
                Ok(t::claim(c, task, &who, 900, false)?.session.id)
            })
            .unwrap()
    }

    fn harness_session(&self, id: &str, agent: &str, mission: Option<&str>) {
        self.db
            .with_conn(|c| {
                sessions::insert(
                    c,
                    &NewSession {
                        id,
                        harness: "claude-code",
                        label: "x",
                        cwd: "/work",
                        mission_id: mission,
                        placement: Placement {
                            host: "local",
                            agent_name: agent,
                            workspace_id: "w",
                            pane_id: "p",
                        },
                        purpose: None,
                    },
                )
            })
            .unwrap();
    }

    fn write(&self, rel: &str, body: &str) -> String {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
        hq_db::search::content_hash(body)
    }

    fn current(&self, rel: &str, hash: &str) -> Option<hq_db::provenance::Provenance> {
        self.db
            .with_conn(|c| hq_db::provenance::current(c, rel, hash))
            .unwrap()
    }

    fn status(&self, task: &str) -> String {
        self.db
            .with_conn(|c| Ok(t::get_task(c, task)?.unwrap().status))
            .unwrap()
    }
}

fn native(session: &str, agent: &str, leases: &[&str]) -> Value {
    json!({
        NATIVE_SESSION_ARG: session,
        NATIVE_AGENT_ARG: agent,
        NATIVE_LEASES_ARG: leases,
    })
}

#[test]
fn a_call_with_no_attested_marker_is_unknown() {
    let fx = Fixture::new();
    assert_eq!(resolve(&json!({"path": "a.md"}), &fx.db), Writer::unknown());
    assert_eq!(resolve(&json!(null), &fx.db), Writer::unknown());
    // A native marker that is blank says nothing.
    assert_eq!(
        resolve(&json!({ NATIVE_SESSION_ARG: "  " }), &fx.db),
        Writer::unknown()
    );
}

#[test]
fn a_connector_grant_is_an_agent_actor_without_a_session() {
    let fx = Fixture::new();
    let writer = resolve(&json!({ CONNECTOR_ARG: "Claude" }), &fx.db);
    assert_eq!(writer.actor_id, Some(connector_actor("Claude")));
    assert_eq!(writer.source_kind, SourceKind::Agent);
    assert_eq!((writer.session_id, writer.task_id), (None, None));
}

#[test]
fn a_connector_marker_outranks_a_native_one() {
    // A connector call that somehow carried both is still a connector call.
    let fx = Fixture::new();
    let mut args = native("s-1", "relay", &[]);
    args[CONNECTOR_ARG] = json!("Claude");
    let writer = resolve(&args, &fx.db);
    assert_eq!(writer.actor_id, Some(connector_actor("Claude")));
    assert_eq!(writer.session_id, None);
}

#[test]
fn a_harness_session_is_named_by_its_agent_and_takes_its_task_from_its_lease() {
    let fx = Fixture::new();
    fx.task("tk-1");
    fx.harness_session("hs-1", "builder", None);
    fx.db
        .with_conn(|c| t::open_for_session(c, "tk-1", "hs-1", "builder", "claude-code", "", ""))
        .unwrap();

    let writer = resolve(&json!({ CALLER_SESSION_ARG: "hs-1" }), &fx.db);
    assert_eq!(writer.actor_id.as_deref(), Some("builder"));
    assert_eq!(writer.source_kind, SourceKind::Agent);
    assert_eq!(writer.session_id.as_deref(), Some("hs-1"));
    assert_eq!(writer.task_id.as_deref(), Some("tk-1"));
}

#[test]
fn a_harness_session_launched_for_a_task_without_a_lease_still_names_the_task() {
    let fx = Fixture::new();
    fx.task("tk-1");
    fx.harness_session("hs-1", "builder", Some("tk-1"));
    let writer = resolve(&json!({ CALLER_SESSION_ARG: "hs-1" }), &fx.db);
    assert_eq!(writer.task_id.as_deref(), Some("tk-1"));
}

#[test]
fn a_harness_session_whose_lease_and_mission_disagree_names_no_task() {
    let fx = Fixture::new();
    fx.task("tk-1");
    fx.task("tk-2");
    fx.harness_session("hs-1", "builder", Some("tk-2"));
    fx.db
        .with_conn(|c| t::open_for_session(c, "tk-1", "hs-1", "builder", "claude-code", "", ""))
        .unwrap();
    let writer = resolve(&json!({ CALLER_SESSION_ARG: "hs-1" }), &fx.db);
    assert_eq!(writer.task_id, None, "two answers is no answer");
    assert_eq!(writer.session_id.as_deref(), Some("hs-1"));
}

#[test]
fn a_harness_session_that_is_not_registered_keeps_its_id_and_no_actor() {
    let fx = Fixture::new();
    let writer = resolve(&json!({ CALLER_SESSION_ARG: "ghost" }), &fx.db);
    assert_eq!(writer.actor_id, None);
    assert_eq!(writer.session_id.as_deref(), Some("ghost"));
    assert_eq!(writer.task_id, None);
}

#[test]
fn a_native_session_takes_its_task_from_the_one_live_lease_it_was_handed() {
    let fx = Fixture::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    let writer = resolve(&native("s-1", "relay", &[&lease]), &fx.db);
    assert_eq!(writer.actor_id.as_deref(), Some("relay"));
    assert_eq!(writer.session_id.as_deref(), Some("s-1"));
    assert_eq!(writer.task_id.as_deref(), Some("tk-1"));
}

#[test]
fn a_native_session_that_holds_no_lease_or_a_dead_one_names_no_task() {
    let fx = Fixture::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    fx.db
        .with_conn(|c| {
            c.execute(
                "UPDATE task_work_sessions SET ended_at = datetime('now') WHERE id = ?1",
                [&lease],
            )?;
            Ok(())
        })
        .unwrap();

    assert_eq!(
        resolve(&native("s-1", "relay", &[&lease]), &fx.db).task_id,
        None
    );
    assert_eq!(resolve(&native("s-1", "relay", &[]), &fx.db).task_id, None);
    assert_eq!(
        resolve(&native("s-1", "relay", &["ws-unknown"]), &fx.db).task_id,
        None
    );
}

#[test]
fn a_native_session_on_two_tasks_at_once_names_neither() {
    let fx = Fixture::new();
    fx.task("tk-1");
    fx.task("tk-2");
    let (a, b) = (fx.lease("tk-1", "relay"), fx.lease("tk-2", "relay-b"));
    let writer = resolve(&native("s-1", "relay", &[&a, &b]), &fx.db);
    assert_eq!(writer.task_id, None, "a guess between tasks would be a lie");
    assert_eq!(writer.session_id.as_deref(), Some("s-1"));
}

#[test]
fn a_write_records_who_wrote_it_and_links_the_note_to_the_task() {
    let fx = Fixture::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    let status_before = fx.status("tk-1");
    let hash = fx.write("Notebooks/a.md", "# a\n");

    fx.recorder()
        .after_write("Notebooks/a.md", &native("s-1", "relay", &[&lease]));

    let p = fx.current("Notebooks/a.md", &hash).expect("recorded");
    assert_eq!(p.actor_id, "relay");
    assert_eq!(p.source_kind, SourceKind::Agent);
    assert_eq!(p.session_id.as_deref(), Some("s-1"));
    assert_eq!(p.task_id.as_deref(), Some("tk-1"));
    assert_eq!(p.review_state, ReviewState::Unreviewed);

    let links = fx.db.with_conn(|c| t::list_task_links(c, "tk-1")).unwrap();
    let note = links
        .iter()
        .find(|l| l.reference == "Notebooks/a.md")
        .expect("a produced link");
    assert_eq!(note.kind, t::LINK_VAULT_NOTE);
    assert_eq!(note.direction, t::DIRECTION_PRODUCED);

    assert_eq!(
        fx.status("tk-1"),
        status_before,
        "writing a note never moves its task"
    );
}

#[test]
fn writing_the_same_note_twice_adds_one_link() {
    let fx = Fixture::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    let args = native("s-1", "relay", &[&lease]);
    fx.write("Notebooks/a.md", "one");
    fx.recorder().after_write("Notebooks/a.md", &args);
    fx.write("Notebooks/a.md", "two");
    fx.recorder().after_write("Notebooks/a.md", &args);

    let links = fx.db.with_conn(|c| t::list_task_links(c, "tk-1")).unwrap();
    assert_eq!(
        links
            .iter()
            .filter(|l| l.reference == "Notebooks/a.md")
            .count(),
        1
    );
}

#[test]
fn an_edit_made_outside_any_tool_stays_unattributed() {
    let fx = Fixture::new();
    let first = fx.write("Notebooks/a.md", "from the agent");
    fx.recorder()
        .after_write("Notebooks/a.md", &native("s-1", "relay", &[]));

    // Someone saves the file in an editor: no tool ran, so nothing recorded it.
    let edited = fx.write("Notebooks/a.md", "from the owner, in an editor");

    assert!(fx.current("Notebooks/a.md", &first).is_some());
    assert_eq!(
        fx.current("Notebooks/a.md", &edited),
        None,
        "the earlier author is not carried onto text they did not write"
    );
}

#[test]
fn two_writers_are_never_credited_with_each_others_versions() {
    let fx = Fixture::new();
    let v1 = fx.write("Notebooks/a.md", "version one");
    fx.recorder()
        .after_write("Notebooks/a.md", &native("s-1", "relay", &[]));
    let v2 = fx.write("Notebooks/a.md", "version two");
    fx.recorder()
        .after_write("Notebooks/a.md", &json!({ CONNECTOR_ARG: "Claude" }));

    assert_eq!(fx.current("Notebooks/a.md", &v1).unwrap().actor_id, "relay");
    assert_eq!(
        fx.current("Notebooks/a.md", &v2).unwrap().actor_id,
        connector_actor("Claude")
    );
}

#[test]
fn the_owner_and_a_daemon_job_are_recorded_under_their_own_kinds() {
    let fx = Fixture::new();
    let a = fx.write("Notebooks/a.md", "from the web app");
    fx.recorder().record("Notebooks/a.md", &Writer::owner());
    let p = fx.current("Notebooks/a.md", &a).unwrap();
    assert_eq!(
        (p.actor_id.as_str(), p.source_kind),
        ("owner", SourceKind::Human)
    );

    let b = fx.write("Notebooks/b.md", "from a job");
    fx.recorder()
        .record("Notebooks/b.md", &Writer::system("cleanup"));
    let p = fx.current("Notebooks/b.md", &b).unwrap();
    assert_eq!(
        (p.actor_id.as_str(), p.source_kind),
        ("system:cleanup", SourceKind::System)
    );
}

#[test]
fn a_note_that_cannot_be_read_is_not_recorded_and_nothing_panics() {
    let fx = Fixture::new();
    fx.recorder()
        .after_write("Notebooks/missing.md", &native("s-1", "relay", &[]));
    let rows = fx
        .db
        .with_conn(|c| hq_db::provenance::history(c, "Notebooks/missing.md", 10))
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn the_async_helper_records_and_does_nothing_without_a_database() {
    let fx = Fixture::new();
    let hash = fx.write("Notebooks/a.md", "text");
    let args = native("s-1", "relay", &[]);

    record_tool_write(None, fx.dir.path(), "Notebooks/a.md", &args).await;
    assert!(fx.current("Notebooks/a.md", &hash).is_none());

    record_tool_write(Some(&fx.db), fx.dir.path(), "Notebooks/a.md", &args).await;
    assert_eq!(
        fx.current("Notebooks/a.md", &hash).unwrap().actor_id,
        "relay"
    );
}

#[test]
fn a_move_carries_the_records_and_credits_only_the_notes_it_really_rewrote() {
    let fx = Fixture::new();
    let rec = fx.recorder();
    let moved_hash = fx.write("Notebooks/old.md", "the note");
    fx.write("Notebooks/links.md", "see [[old]]");
    fx.write("Notebooks/unrelated.md", "no links here");
    rec.record("Notebooks/old.md", &Writer::owner());
    rec.record("Notebooks/links.md", &Writer::owner());
    rec.record("Notebooks/unrelated.md", &Writer::owner());

    let rels: Vec<String> = [
        "Notebooks/old.md",
        "Notebooks/links.md",
        "Notebooks/unrelated.md",
    ]
    .map(String::from)
    .to_vec();
    let before = rec.fingerprint(&rels);

    // The move: the file changes place, one referrer is rewritten, the other is untouched.
    std::fs::create_dir_all(fx.dir.path().join("Notebooks/Archive")).unwrap();
    std::fs::rename(
        fx.dir.path().join("Notebooks/old.md"),
        fx.dir.path().join("Notebooks/Archive/old.md"),
    )
    .unwrap();
    let rewritten = fx.write("Notebooks/links.md", "see [[Archive/old]]");

    rec.after_move(
        "Notebooks/old.md",
        "Notebooks/Archive/old.md",
        &[
            "Notebooks/links.md".to_string(),
            "Notebooks/unrelated.md".to_string(),
        ],
        &before,
        &native("s-1", "relay", &[]),
    );

    // The moved note keeps its author: the text did not change.
    let moved = fx.current("Notebooks/Archive/old.md", &moved_hash).unwrap();
    assert_eq!(moved.actor_id, "owner");
    // The referrer that was rewritten is now a version the mover made.
    assert_eq!(
        fx.current("Notebooks/links.md", &rewritten)
            .unwrap()
            .actor_id,
        "relay"
    );
    // The one whose text did not change gained no record.
    let history = fx
        .db
        .with_conn(|c| hq_db::provenance::history(c, "Notebooks/unrelated.md", 10))
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].actor_id, "owner");
}

#[test]
fn a_delete_keeps_who_wrote_the_note_until_the_tombstone_is_purged() {
    let fx = Fixture::new();
    let hash = fx.write("Notebooks/a.md", "text");
    fx.recorder().record("Notebooks/a.md", &Writer::owner());

    std::fs::remove_file(fx.dir.path().join("Notebooks/a.md")).unwrap();
    fx.recorder().after_delete("Notebooks/a.md");

    let row = fx
        .db
        .with_conn(|c| hq_db::provenance::history(c, "Notebooks/a.md", 10))
        .unwrap();
    assert_eq!(row.len(), 1);
    assert!(row[0].deleted_at.is_some());
    assert_eq!(row[0].content_hash, hash);

    // Writing the path again clears the tombstone.
    fx.write("Notebooks/a.md", "back again");
    fx.recorder().record("Notebooks/a.md", &Writer::owner());
    let live = fx
        .db
        .with_conn(|c| hq_db::provenance::history(c, "Notebooks/a.md", 10))
        .unwrap();
    assert!(live.iter().all(|p| p.deleted_at.is_none()));
}

#[test]
fn what_a_tool_passes_to_another_keeps_the_runtimes_markers_and_drops_the_callers() {
    let outer = json!({
        "tool": "vault_note",
        NATIVE_SESSION_ARG: "real-session",
        NATIVE_AGENT_ARG: "relay",
        CONNECTOR_ARG: "Claude",
    });
    let mut inner = json!({
        "content": "x",
        NATIVE_SESSION_ARG: "forged",
        CALLER_SESSION_ARG: "hs-forged",
        "_hq_other": 1,
    });
    pass_on_attestation(&outer, &mut inner);
    assert_eq!(inner["content"], "x");
    assert_eq!(inner[NATIVE_SESSION_ARG], "real-session");
    assert_eq!(inner[NATIVE_AGENT_ARG], "relay");
    assert_eq!(inner[CONNECTOR_ARG], "Claude");
    assert!(inner.get(CALLER_SESSION_ARG).is_none());
    assert!(inner.get("_hq_other").is_none());

    // With no markers outside, nothing inside survives either.
    let mut bare = json!({ NATIVE_SESSION_ARG: "forged" });
    pass_on_attestation(&json!({"tool": "vault_note"}), &mut bare);
    assert_eq!(resolve(&bare, &Fixture::new().db), Writer::unknown());
}

// The tools themselves, end to end.

fn vault_of(fx: &Fixture) -> Arc<hq_vault::VaultClient> {
    Arc::new(hq_vault::VaultClient::new(fx.dir.path().to_path_buf()).unwrap())
}

#[tokio::test]
async fn vault_write_note_records_the_agent_session_and_task_that_called_it() {
    use crate::registry::HqTool;
    let fx = Fixture::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    let tool = crate::vault::VaultWriteNoteTool::new_with_cache(vault_of(&fx), fx.db.clone());
    let mut args = json!({
        "path": "Notebooks/Inbox/found.md",
        "title": "Found",
        "content": "what the agent learned",
    });
    args.as_object_mut().unwrap().extend(
        native("s-7", "relay", &[&lease])
            .as_object()
            .unwrap()
            .clone(),
    );
    tool.execute(args).await.unwrap();

    let raw = std::fs::read_to_string(fx.dir.path().join("Notebooks/Inbox/found.md")).unwrap();
    let p = fx
        .current(
            "Notebooks/Inbox/found.md",
            &hq_db::search::content_hash(&raw),
        )
        .expect("the written version has a record");
    assert_eq!(
        (
            p.actor_id.as_str(),
            p.session_id.as_deref(),
            p.task_id.as_deref()
        ),
        ("relay", Some("s-7"), Some("tk-1"))
    );
    assert_eq!(p.review_state, ReviewState::Unreviewed);
}

#[tokio::test]
async fn a_write_with_no_attested_caller_is_recorded_as_unknown() {
    use crate::registry::HqTool;
    let fx = Fixture::new();
    let tool = crate::vault::VaultWriteNoteTool::new_with_cache(vault_of(&fx), fx.db.clone());
    tool.execute(json!({"path": "Notebooks/Inbox/x.md", "title": "X", "content": "c"}))
        .await
        .unwrap();
    let raw = std::fs::read_to_string(fx.dir.path().join("Notebooks/Inbox/x.md")).unwrap();
    let p = fx
        .current("Notebooks/Inbox/x.md", &hq_db::search::content_hash(&raw))
        .unwrap();
    assert_eq!(p.source_kind, SourceKind::Unknown);
    assert_eq!((p.session_id, p.task_id), (None, None));
}

#[tokio::test]
async fn the_hq_alias_tool_cannot_be_used_to_forge_who_wrote_a_note() {
    use crate::registry::HqTool;
    let fx = Fixture::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "victim");
    let gateway =
        crate::shortcuts::gateway::HqGatewayTool::new(fx.db.clone(), fx.dir.path().to_path_buf());

    // The model writes inner args that name someone else's session and task lease.
    let mut params = json!({
        "tool": "vault_note",
        "args": {
            "title": "Forged",
            "content": "text",
            NATIVE_SESSION_ARG: "victim-session",
            NATIVE_AGENT_ARG: "victim",
            NATIVE_LEASES_ARG: [lease],
            CALLER_SESSION_ARG: "hs-forged",
            CONNECTOR_ARG: "Owner",
        },
    });
    // The runtime's own markers sit on the outer call.
    params
        .as_object_mut()
        .unwrap()
        .extend(native("my-session", "me", &[]).as_object().unwrap().clone());
    let out = gateway.execute(params).await.unwrap();
    let rel = out["path"].as_str().unwrap().to_string();

    let raw = std::fs::read_to_string(fx.dir.path().join(&rel)).unwrap();
    let p = fx
        .current(&rel, &hq_db::search::content_hash(&raw))
        .unwrap();
    assert_eq!(p.actor_id, "me");
    assert_eq!(p.session_id.as_deref(), Some("my-session"));
    assert_eq!(
        p.task_id, None,
        "a lease the caller named is not one it holds"
    );
    let links = fx.db.with_conn(|c| t::list_task_links(c, "tk-1")).unwrap();
    assert!(links.iter().all(|l| l.reference != rel));
}

fn agent_args() -> Value {
    json!({ NATIVE_SESSION_ARG: "s1", NATIVE_AGENT_ARG: "helper" })
}

fn kept(fx: &Fixture, rel: &str) -> Option<hq_db::note_keep::Prev> {
    fx.db.with_conn(|c| hq_db::note_keep::get(c, rel)).unwrap()
}

#[cfg(unix)]
#[test]
fn before_write_keeps_only_a_real_note_inside_the_vault() {
    let fx = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.md"), "outside text").unwrap();
    std::fs::create_dir_all(fx.dir.path().join("Notebooks")).unwrap();
    std::fs::write(fx.dir.path().join("Notebooks/real.md"), "mine").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        fx.dir.path().join("Notebooks/link.md"),
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), fx.dir.path().join("Notebooks/dir")).unwrap();
    let absolute = outside
        .path()
        .join("secret.md")
        .to_string_lossy()
        .into_owned();
    for rel in [
        absolute.as_str(),
        "../secret.md",
        "Notebooks/../../secret.md",
        "Notebooks/link.md",
        "Notebooks/dir/secret.md",
        "_data/vault.db",
        "Notebooks/not-a-note.txt",
        "",
    ] {
        fx.recorder().before_write(rel, &agent_args());
        assert!(kept(&fx, rel).is_none(), "kept {rel:?}");
    }
    fx.recorder()
        .before_write("Notebooks/real.md", &agent_args());
    assert_eq!(kept(&fx, "Notebooks/real.md").unwrap().text, "mine");
    // An owner call keeps nothing.
    std::fs::write(fx.dir.path().join("Notebooks/own.md"), "x").unwrap();
    fx.recorder().before_write("Notebooks/own.md", &json!({}));
    assert!(kept(&fx, "Notebooks/own.md").is_none());
}

#[cfg(unix)]
#[test]
fn before_write_never_opens_a_fifo_or_reads_past_the_cap() {
    let fx = Fixture::new();
    std::fs::create_dir_all(fx.dir.path().join("Notebooks")).unwrap();
    let fifo = fx.dir.path().join("Notebooks/pipe.md");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    // Opening a FIFO with no writer would block this test forever.
    fx.recorder()
        .before_write("Notebooks/pipe.md", &agent_args());
    assert!(kept(&fx, "Notebooks/pipe.md").is_none());

    // A sparse file far over the cap costs no memory and keeps nothing; an older kept text goes.
    let huge = fx.dir.path().join("Notebooks/huge.md");
    let file = std::fs::File::create(&huge).unwrap();
    file.set_len(4 * 1024 * 1024 * 1024).unwrap();
    fx.db
        .with_conn(|c| hq_db::note_keep::keep(c, "Notebooks/huge.md", "old"))
        .unwrap();
    fx.recorder()
        .before_write("Notebooks/huge.md", &agent_args());
    assert!(kept(&fx, "Notebooks/huge.md").is_none());
}

#[test]
fn a_write_that_did_not_keep_serves_no_stale_text() {
    let fx = Fixture::new();
    std::fs::create_dir_all(fx.dir.path().join("Notebooks")).unwrap();
    let path = fx.dir.path().join("Notebooks/a.md");
    let rel = "Notebooks/a.md";
    std::fs::write(&path, "owner v1").unwrap();
    fx.recorder().record(rel, &Writer::owner());
    fx.recorder().before_write(rel, &agent_args());
    std::fs::write(&path, "hq v1").unwrap();
    fx.recorder().after_write(rel, &agent_args());
    assert_eq!(kept(&fx, rel).unwrap().text, "owner v1");
    // The owner edits; what HQ replaced is older than that, so it is gone.
    std::fs::write(&path, "owner v2").unwrap();
    fx.recorder().record(rel, &Writer::owner());
    assert!(kept(&fx, rel).is_none());
    // An HQ write that skipped the keeping step cannot revive or leave a stale row.
    fx.recorder().before_write(rel, &agent_args());
    std::fs::write(&path, "hq v2").unwrap();
    fx.recorder().after_write(rel, &agent_args());
    assert_eq!(kept(&fx, rel).unwrap().text, "owner v2");
    std::fs::write(&path, "hq v3, no keeping step").unwrap();
    fx.recorder().after_write(rel, &agent_args());
    assert!(kept(&fx, rel).is_none());
}

#[test]
fn failed_write_discards_kept_row_and_does_not_tie_later_non_keeping_write() {
    let fx = Fixture::new();
    std::fs::create_dir_all(fx.dir.path().join("Notebooks")).unwrap();
    let path = fx.dir.path().join("Notebooks/a.md");
    let rel = "Notebooks/a.md";
    std::fs::write(&path, "owner v1").unwrap();
    fx.recorder().record(rel, &Writer::owner());

    fx.recorder().before_write(rel, &agent_args());
    assert!(kept(&fx, rel).is_some());
    // The write failed before landing: discard the kept row.
    fx.recorder().discard_kept(rel, &agent_args());
    assert!(kept(&fx, rel).is_none());

    // An outside edit happens, then a non-keeping HQ write runs: it must not tie "owner v1".
    std::fs::write(&path, "outside edit").unwrap();
    std::fs::write(&path, "hq non-keeping").unwrap();
    fx.recorder().after_write(rel, &agent_args());
    assert!(kept(&fx, rel).is_none());
}
