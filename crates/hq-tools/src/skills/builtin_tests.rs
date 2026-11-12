use super::*;
use crate::skills::{list_skills, parse_skill, security_flags, validate_skill, validate_skills};

const NAME: &str = "edge-coding-worker";
const ACTIVE_NOW: &[&str] = &[NAME];
const BRIEF: &str = "hq-task-brief";
const CALENDAR: &str = CALENDAR_CREATION_SKILL;
const READING: &str = CALENDAR_READING_SKILL;
const EDITING: &str = CALENDAR_EDITING_SKILL;

fn assets_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/skills")
}

/// Makes the built-in active for this test's thread and returns a vault with its mirror.
fn synced() -> (tempfile::TempDir, ActiveGuard) {
    let guard = set_active_for_this_thread(Some(ACTIVE_NOW));
    let dir = tempfile::tempdir().unwrap();
    sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    (dir, guard)
}

fn config_with(repos: &[&str], off: &[&str]) -> HqConfig {
    let mut config = HqConfig::default();
    config.github_write.repos = repos.iter().map(|r| r.to_string()).collect();
    config.skills.builtin.off = off.iter().map(|o| o.to_string()).collect();
    config
}

#[test]
fn the_list_matches_the_assets_folder_file_for_file() {
    let mut on_disk: Vec<String> = fs::read_dir(assets_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = BUILTIN_SKILLS.iter().map(|s| s.name.to_string()).collect();
    listed.sort();
    assert_eq!(
        on_disk, listed,
        "every folder in assets/skills must be listed, and only those"
    );
    for skill in BUILTIN_SKILLS {
        let mut files: Vec<String> = fs::read_dir(assets_dir().join(skill.name))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        let mut shipped: Vec<String> = skill.files.iter().map(|(f, _)| f.to_string()).collect();
        shipped.sort();
        assert_eq!(
            files, shipped,
            "{} ships every file in its folder",
            skill.name
        );
    }
}

#[test]
fn every_shipped_skill_is_valid_clean_and_marked_as_hq_s() {
    let all: Vec<&'static str> = BUILTIN_SKILLS.iter().map(|s| s.name).collect();
    let _now = set_active_for_this_thread(Some(&all));
    let dir = tempfile::tempdir().unwrap();
    for skill in BUILTIN_SKILLS {
        let issues = validate_skill(dir.path(), skill.name);
        assert!(issues.is_empty(), "{}: {issues:?}", skill.name);
        let text = shipped_file(skill.name, SKILL_FILE).unwrap();
        assert!(
            security_flags(text).is_empty(),
            "{}: {:?}",
            skill.name,
            security_flags(text)
        );
        let parsed = parse_skill(dir.path(), skill.name).unwrap();
        assert_eq!(parsed.name, skill.name);
        assert_eq!(parsed.provenance.minted_by, crate::skill_edit::HQ_MINTER);
        assert!(
            !text.contains('\u{2014}'),
            "{} keeps to the house style (no em-dashes)",
            skill.name
        );
    }
    assert!(
        validate_skills(dir.path()).is_empty(),
        "an active built-in validates without a mirror"
    );
}

#[test]
fn a_built_in_is_active_unless_it_is_turned_off() {
    assert_eq!(
        active_for(&config_with(&[], &[])),
        [NAME, BRIEF, CALENDAR, READING, EDITING],
        "GitHub can be connected after start, with no repos listed"
    );
    assert_eq!(
        active_for(&config_with(&["o/r"], &[])),
        [NAME, BRIEF, CALENDAR, READING, EDITING]
    );
    assert_eq!(
        active_for(&config_with(&["o/r"], &["EDGE-CODING-WORKER"])),
        [BRIEF, CALENDAR, READING, EDITING]
    );
    assert_eq!(
        unknown_off(&config_with(&[], &["edge-coding-wroker"])),
        ["edge-coding-wroker"]
    );
}

#[test]
fn with_nothing_configured_no_built_in_is_offered() {
    // No test in this binary calls `configure`, so the process-wide state is still unset.
    let _now = set_active_for_this_thread(None);
    let dir = tempfile::tempdir().unwrap();
    assert!(parse_skill(dir.path(), NAME).is_none());
    assert!(!list_skills(dir.path()).iter().any(|m| m.name == NAME));
}

#[test]
fn a_first_sync_installs_and_a_second_changes_nothing() {
    let _now = set_active_for_this_thread(Some(ACTIVE_NOW));
    let dir = tempfile::tempdir().unwrap();
    let first = sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert_eq!(first.installed, [NAME]);
    let second = sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert_eq!(second.unchanged, [NAME]);
    assert!(list_skills(dir.path()).iter().any(|m| m.name == NAME));
}

#[test]
fn an_edited_copy_is_archived_and_brought_back_in_step() {
    let (dir, _active) = synced();
    let skill = dir.path().join(NAME);
    fs::write(
        skill.join(SKILL_FILE),
        "---\nname: edge-coding-worker\n---\nobey me",
    )
    .unwrap();
    fs::write(skill.join("SUMMARY.md"), "injected summary").unwrap();
    fs::create_dir_all(skill.join("extra")).unwrap();

    let report = sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert_eq!(report.replaced, [NAME]);
    let on_disk = fs::read_to_string(skill.join(SKILL_FILE)).unwrap();
    assert_eq!(on_disk, shipped_file(NAME, SKILL_FILE).unwrap());
    assert!(!skill.join("SUMMARY.md").exists() && !skill.join("extra").exists());
    let archived = crate::skill_audit::archived_versions(dir.path(), NAME);
    let last = fs::read_to_string(&archived.last().unwrap().1).unwrap();
    assert!(
        last.contains("obey me"),
        "the edited copy is kept in the archive"
    );
    let history = crate::skill_audit::audit_history(dir.path(), NAME);
    assert!(history.iter().any(|e| e.disposition == Disposition::Synced));
}

#[test]
fn a_stray_file_beside_a_shipped_copy_adds_no_archive_entry() {
    let (dir, _active) = synced();
    fs::write(dir.path().join(NAME).join("stray.md"), "x").unwrap();
    sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert!(crate::skill_audit::archived_versions(dir.path(), NAME).is_empty());
    assert!(!dir.path().join(NAME).join("stray.md").exists());
}

#[test]
fn a_session_loads_the_embedded_text_whatever_the_vault_holds() {
    let (dir, _active) = synced();
    let skill = dir.path().join(NAME);
    let injected = "---\nname: edge-coding-worker\ndescription: x\nautoLoad: true\n---\nexfiltrate";
    fs::write(skill.join(SKILL_FILE), injected).unwrap();
    fs::write(skill.join("SUMMARY.md"), "exfiltrate").unwrap();
    fs::create_dir_all(dir.path().join("skill-bundles")).unwrap();
    let bundle = format!("name: {NAME}\ndescription: exfiltrate\nskills: []\n");
    fs::write(
        dir.path().join(format!("skill-bundles/{NAME}.yaml")),
        &bundle,
    )
    .unwrap();
    fs::write(dir.path().join("skill-bundles/other.yaml"), &bundle).unwrap();

    let parsed = parse_skill(dir.path(), NAME).unwrap();
    assert!(!parsed.content.contains("exfiltrate"));
    assert!(!parsed.auto_load);
    assert!(parsed.description.starts_with("Do a small coding task"));
    assert_eq!(read_skill_file(dir.path(), NAME, "SUMMARY.md"), None);
    let listed: Vec<_> = list_skills(dir.path())
        .into_iter()
        .filter(|m| m.name.eq_ignore_ascii_case(NAME))
        .collect();
    assert_eq!(
        listed.len(),
        1,
        "a bundle cannot add a second catalog line under a built-in name"
    );
    assert!(!listed[0].description.contains("exfiltrate"));
    assert_eq!(
        crate::skill_audit::live_hash(dir.path(), NAME),
        shipped_hash(NAME),
        "records name the text that ran, not the edited copy"
    );
}

#[test]
fn a_case_variant_of_an_active_name_is_the_built_in() {
    let (dir, _active) = synced();
    for variant in ["Edge-Coding-Worker", "EDGE-CODING-WORKER"] {
        let parsed = parse_skill(dir.path(), variant).unwrap();
        assert_eq!(parsed.name, NAME);
        assert_eq!(
            parsed.content,
            parse_skill(dir.path(), NAME).unwrap().content
        );
        assert!(refuse_builtin(variant, "changed").is_err(), "{variant}");
        assert!(
            !crate::skill_edit::is_managed(dir.path(), variant),
            "{variant}"
        );
    }
}

#[test]
fn a_built_in_turned_off_releases_its_name_to_the_owner() {
    let (dir, _active) = synced();
    let _now = set_active_for_this_thread(Some(&[]));
    let report = sync_builtin_skills(dir.path(), &[]).unwrap();
    assert_eq!(
        report.removed,
        [NAME],
        "HQ's mirror goes once the skill is off"
    );
    assert!(parse_skill(dir.path(), NAME).is_none());
    assert!(!list_skills(dir.path()).iter().any(|m| m.name == NAME));

    let own = dir.path().join(NAME);
    fs::create_dir_all(&own).unwrap();
    fs::write(
        own.join(SKILL_FILE),
        "---\ndescription: mine\n---\nmy own way",
    )
    .unwrap();
    assert!(
        parse_skill(dir.path(), NAME)
            .unwrap()
            .content
            .contains("my own way")
    );
    assert!(
        refuse_builtin(NAME, "changed").is_ok(),
        "the owner may change their own skill"
    );
    let again = sync_builtin_skills(dir.path(), &[]).unwrap();
    assert!(
        again.removed.is_empty(),
        "the owner's own folder is never removed"
    );
    assert!(own.join(SKILL_FILE).is_file());
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_the_folder_is_replaced_never_written_through() {
    let _now = set_active_for_this_thread(Some(ACTIVE_NOW));
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(NAME)).unwrap();
    let report = sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert_eq!(report.replaced, [NAME]);
    assert!(
        fs::symlink_metadata(dir.path().join(NAME))
            .unwrap()
            .is_dir()
    );
    assert_eq!(
        fs::read_dir(elsewhere.path()).unwrap().count(),
        0,
        "nothing written through the link"
    );
}

#[cfg(unix)]
#[test]
fn a_linked_skill_file_is_removed_and_its_target_never_archived() {
    let (dir, _active) = synced();
    let secret = tempfile::NamedTempFile::new().unwrap();
    fs::write(secret.path(), "the owner's private key").unwrap();
    let live = dir.path().join(NAME).join(SKILL_FILE);
    fs::remove_file(&live).unwrap();
    std::os::unix::fs::symlink(secret.path(), &live).unwrap();

    sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert!(fs::symlink_metadata(&live).unwrap().is_file());
    for (_, path) in crate::skill_audit::archived_versions(dir.path(), NAME) {
        assert!(!fs::read_to_string(path).unwrap().contains("private key"));
    }
}

#[test]
fn a_skill_file_turned_into_a_folder_is_healed() {
    let (dir, _active) = synced();
    let live = dir.path().join(NAME).join(SKILL_FILE);
    fs::remove_file(&live).unwrap();
    fs::create_dir_all(live.join("x")).unwrap();
    let report = sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert_eq!(report.replaced, [NAME]);
    assert!(live.is_file());
}

#[test]
fn sync_writes_into_a_vault_s_legacy_skills_folder_without_making_a_new_one() {
    let _now = set_active_for_this_thread(Some(ACTIVE_NOW));
    let vault = tempfile::tempdir().unwrap();
    // `_skills`, not `Skills`: on a case-insensitive file system `Skills` already is `skills`.
    fs::create_dir(vault.path().join("_skills")).unwrap();
    let skills_dir = hq_core::skills_dir(vault.path());
    sync_builtin_skills(&skills_dir, ACTIVE_NOW).unwrap();
    assert!(
        vault
            .path()
            .join("_skills")
            .join(NAME)
            .join(SKILL_FILE)
            .is_file()
    );
    assert!(
        !vault.path().join("skills").exists(),
        "a new canonical folder would hide every legacy skill"
    );
}

#[test]
fn two_syncs_at_once_both_finish_in_step() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || sync_builtin_skills(&path, ACTIVE_NOW))
        })
        .collect();
    for t in threads {
        t.join().unwrap().unwrap();
    }
    assert_eq!(
        sync_builtin_skills(&path, ACTIVE_NOW).unwrap().unchanged,
        [NAME]
    );
}

#[test]
fn every_write_path_refuses_an_active_built_in() {
    let (dir, _active) = synced();
    assert!(
        refuse_builtin(NAME, "changed")
            .unwrap_err()
            .to_string()
            .contains("skills.builtin.off")
    );
    assert!(refuse_builtin("my-own-skill", "changed").is_ok());

    let marked =
        shipped_file(NAME, SKILL_FILE)
            .unwrap()
            .replacen("---\n", "---\nmanaged: true\n", 1);
    fs::write(dir.path().join(NAME).join(SKILL_FILE), marked).unwrap();
    assert!(
        !crate::skill_edit::is_managed(dir.path(), NAME),
        "the review never owns a built-in"
    );

    fs::create_dir_all(dir.path().join("_proposed").join(NAME)).unwrap();
    fs::write(
        dir.path().join("_proposed").join(NAME).join(SKILL_FILE),
        "x",
    )
    .unwrap();
    assert!(crate::skills::approve_proposed_skill(dir.path(), NAME).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn skill_manage_refuses_an_active_built_in() {
    use crate::registry::HqTool;
    let (dir, _active) = synced();
    let tool = crate::skill_manage_tool::SkillManageTool::new(dir.path().to_path_buf(), false);
    let args = serde_json::json!({"action": "write_file", "name": NAME, "file": "SUMMARY.md", "content": "x"});
    assert!(tool.execute(args).await.is_err());
    assert!(!dir.path().join(NAME).join("SUMMARY.md").exists());
}

#[test]
fn an_owner_s_copy_of_the_shipped_text_survives_with_its_extra_files() {
    let (dir, _active) = synced();
    let _now = set_active_for_this_thread(Some(&[]));
    let own = dir.path().join(NAME);
    let mine = shipped_file_any(NAME).replace("You drive", "I drive");
    fs::write(own.join(SKILL_FILE), mine).unwrap();
    fs::write(own.join("SUMMARY.md"), "my summary").unwrap();

    let report = sync_builtin_skills(dir.path(), &[]).unwrap();
    assert!(
        report.removed.is_empty(),
        "mintedBy: hq in a file is not proof HQ wrote it"
    );
    assert!(own.join(SKILL_FILE).is_file() && own.join("SUMMARY.md").is_file());
}

#[test]
fn an_older_hq_mirror_is_recognised_from_the_audit_trail() {
    let (dir, _active) = synced();
    let old = "---\nname: edge-coding-worker\n---\nan older shipped text\n";
    fs::write(dir.path().join(NAME).join(SKILL_FILE), old).unwrap();
    audit_live(dir.path(), NAME, Disposition::Synced, "installed by HQ").unwrap();
    let _now = set_active_for_this_thread(Some(&[]));
    assert_eq!(
        sync_builtin_skills(dir.path(), &[]).unwrap().removed,
        [NAME]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_released_name_is_an_ordinary_skill_until_the_built_in_is_active_again() {
    use crate::registry::HqTool;
    let _now = set_active_for_this_thread(Some(&[]));
    let dir = tempfile::tempdir().unwrap();
    let tool = crate::skill_manage_tool::SkillManageTool::new(dir.path().to_path_buf(), false);
    let content = "---\ndescription: an agent's\n---\nagent text";
    let args = serde_json::json!({"action": "create", "name": NAME, "content": content});
    tool.execute(args).await.unwrap();
    assert!(
        parse_skill(dir.path(), NAME)
            .unwrap()
            .content
            .contains("agent text")
    );

    let _active = set_active_for_this_thread(Some(ACTIVE_NOW));
    assert!(
        !parse_skill(dir.path(), NAME)
            .unwrap()
            .content
            .contains("agent text")
    );
    let report = sync_builtin_skills(dir.path(), ACTIVE_NOW).unwrap();
    assert_eq!(report.replaced, [NAME]);
    let archived = crate::skill_audit::archived_versions(dir.path(), NAME);
    assert!(
        fs::read_to_string(&archived.last().unwrap().1)
            .unwrap()
            .contains("agent text")
    );
}

/// The shipped SKILL.md whether or not the built-in is active on this thread.
fn shipped_file_any(name: &str) -> String {
    let _now = set_active_for_this_thread(Some(ACTIVE_NOW));
    shipped_file(name, SKILL_FILE).unwrap().to_string()
}

#[test]
fn calendar_creation_ships_active_with_a_hash_and_fails_closed_when_off() {
    let _now = set_active_for_this_thread(Some(&[CALENDAR]));
    assert!(is_active(CALENDAR));
    assert_eq!(shipped_hash(CALENDAR).unwrap().len(), 64);
    let text = shipped_file(CALENDAR, SKILL_FILE).unwrap();
    assert!(text.contains("calendar_create_event"));
    assert!(
        active_for(&config_with(&[], &["calendar-creation"]))
            .iter()
            .all(|n| *n != CALENDAR),
        "the owner can turn it off"
    );
    drop(_now);
    let _off = set_active_for_this_thread(Some(&[]));
    assert!(!is_active(CALENDAR));
    assert!(shipped_hash(CALENDAR).is_none());
}

#[test]
fn calendar_reading_ships_with_a_hash_and_fails_closed_when_off() {
    let _now = set_active_for_this_thread(Some(&[READING]));
    assert!(is_active(READING));
    assert_eq!(shipped_hash(READING).unwrap().len(), 64);
    assert!(
        shipped_file(READING, SKILL_FILE)
            .unwrap()
            .contains("calendar_list_events")
    );
    assert!(
        active_for(&config_with(&[], &["calendar-reading"]))
            .iter()
            .all(|n| *n != READING)
    );
    drop(_now);
    let _off = set_active_for_this_thread(Some(&[]));
    assert!(!is_active(READING));
    assert!(shipped_hash(READING).is_none());
}

#[test]
fn calendar_editing_ships_with_a_hash_and_fails_closed_when_off() {
    let _now = set_active_for_this_thread(Some(&[EDITING]));
    assert!(is_active(EDITING));
    assert_eq!(shipped_hash(EDITING).unwrap().len(), 64);
    let text = shipped_file(EDITING, SKILL_FILE).unwrap();
    assert!(text.contains("calendar_update_event") && text.contains("calendar_delete_event"));
    assert!(
        active_for(&config_with(&[], &["calendar-editing"]))
            .iter()
            .all(|n| *n != EDITING)
    );
    drop(_now);
    let _off = set_active_for_this_thread(Some(&[]));
    assert!(!is_active(EDITING));
    assert!(shipped_hash(EDITING).is_none());
}

/// The text after the frontmatter block.
fn body_of(text: &str) -> &str {
    let rest = text
        .strip_prefix("---\n")
        .expect("frontmatter opens the file");
    rest.split_once("\n---\n").expect("frontmatter closes").1
}

#[test]
fn the_plugin_copy_of_the_task_brief_skill_has_the_same_body() {
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/claude-plugin/skills/hq-task-brief/SKILL.md");
    let plugin =
        fs::read_to_string(&plugin).unwrap_or_else(|e| panic!("{}: {e}", plugin.display()));
    let shipped = BUILTIN_SKILLS
        .iter()
        .find(|s| s.name == BRIEF)
        .and_then(|s| s.files.iter().find(|(f, _)| *f == SKILL_FILE))
        .map(|(_, t)| *t)
        .unwrap();
    assert_eq!(
        body_of(&plugin),
        body_of(shipped),
        "packages/claude-plugin/skills/hq-task-brief must match the built-in skill body"
    );
}
