use super::*;
use crate::provenance::{
    NATIVE_AGENT_ARG, NATIVE_LEASES_ARG, NATIVE_SESSION_ARG, Recorder, Writer,
};
use crate::registry::HqTool;
use crate::vault::{Reviewer, review_note};
use hq_db::harness_sessions_registry::{NewSession, Placement};
use std::sync::Arc;

struct Fx {
    db: Arc<Database>,
    vault: Arc<VaultClient>,
    dir: tempfile::TempDir,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            t::create_initiative(c, "in-1", "personal", None, "Work", "work", "WORK")?;
            t::create_initiative(c, "in-2", "personal", None, "Other", "other", "OTH")?;
            Ok(())
        })
        .unwrap();
        Self {
            vault: Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap()),
            db,
            dir,
        }
    }

    fn task_in(&self, initiative: &str, id: &str) {
        self.db
            .with_conn(|c| {
                t::create_task(
                    c,
                    id,
                    initiative,
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

    fn task(&self, id: &str) {
        self.task_in("in-1", id);
    }

    fn set_status(&self, id: &str, status: &str) {
        self.db
            .with_conn(|c| {
                t::update_task(
                    c,
                    id,
                    &t::TaskPatch {
                        status: Some(status.to_string()),
                        ..Default::default()
                    },
                    None,
                )
                .map(|_| ())
            })
            .unwrap();
    }

    fn status(&self, id: &str) -> String {
        self.db
            .with_conn(|c| Ok(t::get_task(c, id)?.unwrap().status))
            .unwrap()
    }

    fn lease(&self, task: &str, actor: &str) -> String {
        self.db
            .with_conn(|c| {
                let who = t::LeaseIdentity {
                    actor,
                    harness: "native",
                    external_session_ref: actor,
                    ..Default::default()
                };
                Ok(t::claim(c, task, &who, 900, false)?.session.id)
            })
            .unwrap()
    }

    fn session(&self, id: &str, agent: &str, mission: Option<&str>, status: &str) {
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
                )?;
                c.execute(
                    "UPDATE harness_sessions SET status = ?2 WHERE id = ?1",
                    rusqlite::params![id, status],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn recorder(&self) -> Recorder {
        Recorder::new(self.db.clone(), self.dir.path().to_path_buf())
    }

    /// A note written by a native agent session working under `lease`.
    fn agent_note(&self, rel: &str, body: &str, session: &str, agent: &str, lease: &str) {
        self.write(rel, body);
        self.recorder().after_write(
            rel,
            &json!({
                NATIVE_SESSION_ARG: session,
                NATIVE_AGENT_ARG: agent,
                NATIVE_LEASES_ARG: [lease],
            }),
        );
    }

    fn view(&self, task: &str) -> Value {
        task_activity(&self.db, &self.vault, task, Options::default()).unwrap()
    }
}

fn gap_kinds(view: &Value) -> Vec<String> {
    view["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["kind"].as_str().unwrap().to_string())
        .collect()
}

fn gap_for<'a>(view: &'a Value, kind: &str, subject: &str) -> Option<&'a Value> {
    view["gaps"]
        .as_array()?
        .iter()
        .find(|g| g["kind"] == kind && g["subject"] == subject)
}

#[test]
fn a_multi_agent_task_shows_status_contributors_review_state_and_attribution_gaps() {
    let fx = Fx::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    // Agent one holds the lease and writes two notes; agent two works in a spawned session
    // launched for the task and is the one that wrote nothing yet.
    fx.agent_note("Notebooks/plan.md", "plan v1", "s-1", "relay", &lease);
    fx.agent_note("Notebooks/notes.md", "notes", "s-1", "relay", &lease);
    fx.session("hs-9", "builder", Some("tk-1"), "running");
    // A third note is edited by hand after its last recorded write.
    fx.agent_note("Notebooks/edited.md", "before", "s-1", "relay", &lease);
    fx.write("Notebooks/edited.md", "after, by hand");
    // A fourth is linked to the task but nothing ever recorded its author.
    fx.write("Notebooks/found.md", "from somewhere");
    fx.db
        .with_conn(|c| {
            t::add_task_link(
                c,
                "tk-1",
                t::LINK_VAULT_NOTE,
                "Notebooks/found.md",
                "",
                None,
                "test",
            )
        })
        .unwrap();
    // The owner accepts one of the agent's notes.
    review_note(
        &fx.db,
        &fx.vault,
        "Notebooks/plan.md",
        None,
        ReviewState::Accepted,
        Some("checked"),
        &Reviewer::owner(),
    )
    .unwrap();

    let v = fx.view("tk-1");

    assert_eq!(v["scope"], "task");
    assert_eq!(v["task"]["status"], "in_progress");
    assert_eq!(v["holders"][0]["actor"], "relay");
    assert_eq!(v["sessions"][0]["id"], "hs-9");
    assert_eq!(v["sessions"][0]["running"], true);
    assert_eq!(v["sessions"][0]["via"], json!(["mission"]));

    let relay = v["contributors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["actor"] == "relay")
        .expect("the writing agent is a contributor");
    // The hand-edited note is not credited to the agent: what is on disk is not what it wrote.
    assert_eq!(relay["notes"], 2);
    assert_eq!(relay["leases"], 1);

    assert_eq!(v["review_counts"]["accepted"], 1);
    assert_eq!(v["review_counts"]["unreviewed"], 1);
    assert!(v["review_counts"].get("accepted").is_some());
    let plan = v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == "Notebooks/plan.md")
        .unwrap();
    assert_eq!(plan["review_state"], "accepted");
    assert_eq!(plan["reviewed_by"], "owner");
    assert_eq!(plan["review_reason"], "checked");
    assert_eq!(plan["actor"], "relay");
    assert_eq!(plan["session"], "s-1");
    assert_eq!(plan["task"], "tk-1");

    assert!(gap_for(&v, "note_changed_outside_tools", "Notebooks/edited.md").is_some());
    assert!(gap_for(&v, "note_unattributed", "Notebooks/found.md").is_some());
    assert_eq!(v["gaps_total"], v["gaps"].as_array().unwrap().len());
}

#[test]
fn file_writes_never_move_a_task_or_make_it_look_finished() {
    let fx = Fx::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    assert_eq!(fx.status("tk-1"), "in_progress");

    fx.agent_note("Notebooks/a.md", "done, really", "s-1", "relay", &lease);
    fx.agent_note("Notebooks/b.md", "FINISHED", "s-1", "relay", &lease);
    let v = fx.view("tk-1");

    assert_eq!(fx.status("tk-1"), "in_progress");
    assert_eq!(v["task"]["status"], "in_progress");
    assert_eq!(v["awaiting_review"], false);
    assert_eq!(v["task"]["completed_at"], Value::Null);
}

#[test]
fn a_session_ending_does_not_complete_or_release_the_task() {
    let fx = Fx::new();
    fx.task("tk-1");
    fx.session("hs-1", "builder", Some("tk-1"), "exited");
    fx.set_status("tk-1", "in_progress");

    let v = fx.view("tk-1");

    assert_eq!(v["task"]["status"], "in_progress");
    assert_eq!(v["sessions"][0]["running"], false);
    // What the view can say is that nothing holds it now, and it says that as a gap.
    assert!(
        gap_for(
            &v,
            "in_progress_without_a_holder",
            v["task"]["display_id"].as_str().unwrap()
        )
        .is_some()
    );
    assert_eq!(v["holders"], json!([]));
}

#[test]
fn decisions_and_findings_are_split_by_what_has_been_decided_about_them() {
    let fx = Fx::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    for (name, ty) in [
        ("a", "decision"),
        ("b", "decision"),
        ("c", "finding"),
        ("d", "note"),
    ] {
        fx.agent_note(
            &format!("Notebooks/{name}.md"),
            &format!("---\ntype: {ty}\n---\nbody {name}"),
            "s-1",
            "relay",
            &lease,
        );
    }
    let owner = Reviewer::owner();
    review_note(
        &fx.db,
        &fx.vault,
        "Notebooks/a.md",
        None,
        ReviewState::Accepted,
        None,
        &owner,
    )
    .unwrap();
    review_note(
        &fx.db,
        &fx.vault,
        "Notebooks/c.md",
        None,
        ReviewState::Disputed,
        Some("wrong year"),
        &owner,
    )
    .unwrap();

    let v = fx.view("tk-1");
    let split = &v["decisions_and_findings"];

    let paths = |state: &str| -> Vec<String> {
        split[state]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|n| n["path"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(paths("accepted"), ["Notebooks/a.md"]);
    assert_eq!(paths("unreviewed"), ["Notebooks/b.md"]);
    assert_eq!(paths("disputed"), ["Notebooks/c.md"]);
    // A note that does not call itself a decision or a finding is just a note.
    assert!(
        split
            .as_object()
            .unwrap()
            .values()
            .flat_map(|v| v.as_array().unwrap())
            .all(|n| n["path"] != "Notebooks/d.md")
    );
}

#[test]
fn open_questions_and_blockers_appear_only_when_someone_recorded_them() {
    let fx = Fx::new();
    fx.task("tk-1");
    let v = fx.view("tk-1");
    assert_eq!(v["open_questions"], json!([]));
    assert_eq!(v["blocker"], Value::Null);

    // A checkpoint with open questions, and a blocked status with a reason.
    let lease = fx.lease("tk-1", "relay");
    fx.db
        .with_conn(|c| {
            t::add_checkpoint(
                c,
                "tk-1",
                Some(&lease),
                "relay",
                &t::Checkpoint {
                    summary: "half way".into(),
                    next_step: "finish the import".into(),
                    open_questions: "Which tax year?".into(),
                    files: vec![],
                },
            )
            .map(|_| ())
        })
        .unwrap();
    let v = fx.view("tk-1");
    assert_eq!(v["open_questions"][0]["text"], "Which tax year?");
    assert_eq!(v["open_questions"][0]["source"], "checkpoint");
    assert_eq!(v["checkpoint"]["next_step"], "finish the import");
}

#[test]
fn a_blocked_task_reports_its_recorded_reason() {
    let fx = Fx::new();
    fx.task("tk-1");
    fx.db
        .with_conn(|c| {
            t::update_task(
                c,
                "tk-1",
                &t::TaskPatch {
                    status: Some("blocked".into()),
                    blocked_reason: Some(Some("waiting on the bank".into())),
                    ..Default::default()
                },
                None,
            )
            .map(|_| ())
        })
        .unwrap();
    let v = fx.view("tk-1");
    assert_eq!(v["blocker"]["reason"], "waiting on the bank");
}

#[test]
fn every_source_reports_its_own_freshness_and_an_empty_one_is_named() {
    let fx = Fx::new();
    fx.task("tk-1");
    let v = fx.view("tk-1");
    assert!(v["freshness"]["task"].is_string());
    assert!(v["generated_at"].is_string());
    let empty: Vec<&str> = v["sources_with_nothing"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for source in ["checkpoint", "leases", "sessions", "notes"] {
        assert!(
            empty.contains(&source),
            "{source} should be reported empty: {empty:?}"
        );
    }

    let lease = fx.lease("tk-1", "relay");
    fx.agent_note("Notebooks/a.md", "x", "s-1", "relay", &lease);
    let v = fx.view("tk-1");
    assert!(v["freshness"]["leases"].is_string());
    assert!(v["freshness"]["notes"].is_string());
}

#[test]
fn a_note_in_a_private_folder_is_never_listed() {
    let fx = Fx::new();
    fx.task("tk-1");
    fx.write("_system/SOUL.md", "private");
    fx.db
        .with_conn(|c| {
            t::add_task_link(
                c,
                "tk-1",
                t::LINK_VAULT_NOTE,
                "_system/SOUL.md",
                "",
                None,
                "test",
            )
        })
        .unwrap();
    let v = fx.view("tk-1");
    assert_eq!(v["notes"], json!([]));
}

#[test]
fn the_note_limit_is_honoured_and_the_cut_is_reported() {
    let fx = Fx::new();
    fx.task("tk-1");
    let lease = fx.lease("tk-1", "relay");
    for i in 0..5 {
        fx.agent_note(&format!("Notebooks/n{i}.md"), "x", "s-1", "relay", &lease);
    }
    let v = task_activity(
        &fx.db,
        &fx.vault,
        "tk-1",
        Options::with_notes_limit(Some(2)),
    )
    .unwrap();
    assert_eq!(v["notes"].as_array().unwrap().len(), 2);
    assert_eq!(v["notes_total"], 5);
    assert!(gap_kinds(&v).contains(&"notes_not_listed".to_string()));
}

#[test]
fn a_session_the_registry_has_never_heard_of_is_a_gap_not_a_guess() {
    let fx = Fx::new();
    fx.task("tk-1");
    fx.db
        .with_conn(|c| t::add_task_link(c, "tk-1", t::LINK_SESSION, "hs-ghost", "", None, "test"))
        .unwrap();
    let v = fx.view("tk-1");
    assert_eq!(v["sessions"][0]["registered"], false);
    assert!(gap_for(&v, "session_not_registered", "hs-ghost").is_some());
}

#[test]
fn the_view_of_an_initiative_summarises_its_tasks_and_nothing_from_other_initiatives() {
    let fx = Fx::new();
    fx.task_in("in-1", "tk-1");
    fx.task_in("in-1", "tk-2");
    fx.task_in("in-2", "tk-3");
    let lease = fx.lease("tk-1", "relay");
    fx.agent_note("Notebooks/a.md", "x", "s-1", "relay", &lease);
    fx.set_status("tk-2", "ready_for_review");
    let other = fx.lease("tk-3", "elsewhere");
    fx.agent_note(
        "Notebooks/secret.md",
        "other initiative",
        "s-2",
        "elsewhere",
        &other,
    );

    let v = initiative_activity(&fx.db, &fx.vault, "in-1", Options::default()).unwrap();

    assert_eq!(v["scope"], "initiative");
    assert_eq!(v["tasks_shown"], 2);
    assert_eq!(v["status_counts"]["in_progress"], 1);
    assert_eq!(v["status_counts"]["ready_for_review"], 1);
    assert_eq!(v["awaiting_review"].as_array().unwrap().len(), 1);
    assert_eq!(v["recent_notes"][0]["path"], "Notebooks/a.md");
    assert_eq!(v["recent_notes"][0]["task_display_id"], "WORK-001");
    let text = v.to_string();
    assert!(
        !text.contains("secret.md"),
        "another initiative's note leaked"
    );
    assert!(
        !text.contains("elsewhere"),
        "another initiative's agent leaked"
    );
    assert!(
        v["contributors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["actor"] == "relay")
    );
}

#[tokio::test]
async fn the_tool_needs_exactly_one_target_and_reports_an_unknown_one() {
    let fx = Fx::new();
    fx.task("tk-1");
    let tool = TaskActivityTool {
        vault: fx.vault.clone(),
        db: fx.db.clone(),
    };
    assert!(tool.execute(json!({})).await.is_err());
    assert!(
        tool.execute(json!({"task_id": "tk-1", "initiative_id": "in-1"}))
            .await
            .is_err()
    );
    assert!(tool.execute(json!({"task_id": "nope"})).await.is_err());
    assert!(
        tool.execute(json!({"initiative_id": "nope"}))
            .await
            .is_err()
    );
    assert!(tool.is_read_only());
    assert_eq!(
        tool.execute(json!({"task_id": "tk-1"})).await.unwrap()["scope"],
        "task"
    );
    assert_eq!(
        tool.execute(json!({"initiative_id": "in-1"}))
            .await
            .unwrap()["scope"],
        "initiative"
    );
}

#[test]
fn nothing_in_the_view_is_a_token_or_a_secret() {
    let fx = Fx::new();
    fx.task("tk-1");
    fx.lease("tk-1", "relay");
    let text = fx.view("tk-1").to_string();
    assert!(!text.contains("hql_"), "a lease token appeared");
    assert!(!text.contains("token"), "the word token appeared: {text}");
    let _ = Writer::unknown();
}
