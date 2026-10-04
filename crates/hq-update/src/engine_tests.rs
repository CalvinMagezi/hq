use crate::cli::{EXIT_UP_TO_DATE, EXIT_UPDATE_AVAILABLE, check_exit_code};
use crate::engine::{ApplyOptions, Outcome};
use crate::error::UpdateError;
use crate::lock::UpdateLock;
use crate::state::{Layout, State};
use crate::testkit::*;
use std::fs;

fn applied(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Applied { .. })
}

fn fixture_with_release() -> Fixture {
    let f = Fixture::new();
    f.publish_and_point(&ReleaseSpec::new("0.9.1", NEW_SHA));
    f
}

#[tokio::test]
async fn happy_path_swaps_binary_and_web_and_runs_hooks() {
    let f = fixture_with_release();
    let outcome = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(applied(&outcome), "{outcome:?}");
    assert!(f.installed_binary().contains(NEW_SHA));
    assert_eq!(f.web_index(), "web 0.9.1 index.html");
    let layout = Layout::from_config(&f.cfg);
    assert!(
        fs::read_to_string(layout.backup(1))
            .unwrap()
            .contains(OLD_SHA)
    );
    assert_eq!(
        fs::read_to_string(layout.prev_web().join("index.html")).unwrap(),
        "web 0.9.0 index.html"
    );
    assert!(!layout.staged_binary().exists() && !layout.staged_web().exists());
    let events = f.world.events();
    let pos = |needle: &str| {
        events
            .iter()
            .position(|e| e.starts_with(needle))
            .unwrap_or_else(|| panic!("{needle} in {events:?}"))
    };
    assert!(pos("notify:pre") < pos("db-snapshot"));
    assert!(pos("db-snapshot") < pos("restart"));
    assert!(pos("restart") < pos("install --upgrade"));
    assert!(pos("install --upgrade") < pos("notify:post-ok"));
    let state = State::load(&layout).unwrap();
    assert_eq!(state.history.len(), 1);
    assert!(state.history[0].has_web);
    assert!(state.history[0].db_snapshot.as_ref().unwrap().is_file());
}

#[tokio::test]
async fn upgrade_hook_failure_is_reported_but_not_fatal() {
    let f = fixture_with_release();
    *f.world.upgrade_fails.lock().unwrap() = true;
    let outcome = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Applied {
                upgrade_hook_ok: false,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert!(f.installed_binary().contains(NEW_SHA));
}

#[tokio::test]
async fn up_to_date_and_downgrade_are_noops() {
    let f = Fixture::new();
    f.publish_and_point(&ReleaseSpec::new("0.9.0", OLD_SHA));
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(o, Outcome::UpToDate { note: None, .. }), "{o:?}");

    let f = Fixture::new();
    f.publish_and_point(&ReleaseSpec::new("0.8.0", "ccccccc3333"));
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    let Outcome::UpToDate {
        note: Some(note), ..
    } = o
    else {
        panic!("{o:?}")
    };
    assert!(note.contains("refusing downgrade"), "{note}");
    assert!(f.installed_binary().contains(OLD_SHA));
    assert!(f.world.events().iter().all(|e| !e.starts_with("restart")));
}

#[tokio::test]
async fn check_reports_availability_and_exit_codes() {
    let f = fixture_with_release();
    let report = f.engine().check(None).await.unwrap();
    assert!(report.update_available);
    assert_eq!(report.available.as_deref(), Some("0.9.1"));
    assert_eq!(check_exit_code(&report), EXIT_UPDATE_AVAILABLE);
    assert_eq!(EXIT_UPDATE_AVAILABLE, 10);

    let report = f.engine_as("0.9.1", NEW_SHA).check(None).await.unwrap();
    assert!(!report.update_available);
    assert_eq!(check_exit_code(&report), EXIT_UP_TO_DATE);

    let report = f.engine_as("0.9.5", NEW_SHA).check(None).await.unwrap();
    assert!(!report.update_available);
    assert!(report.note.unwrap().contains("downgrades need --pin"));
    // check never modifies anything
    assert!(f.installed_binary().contains(OLD_SHA));
}

#[tokio::test]
async fn dry_run_changes_nothing() {
    let f = fixture_with_release();
    let o = f
        .engine()
        .apply(&ApplyOptions {
            dry_run: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(matches!(o, Outcome::DryRun { .. }));
    assert!(f.installed_binary().contains(OLD_SHA));
    assert!(f.world.events().is_empty());
    assert!(!Layout::from_config(&f.cfg).state_file().exists());
}

#[tokio::test]
async fn tampered_manifest_is_refused() {
    let f = fixture_with_release();
    let url = format!("{REPO_BASE}/v0.9.1/manifest.json");
    let mut body = f.http.get_raw(&url).unwrap();
    let at = body.iter().position(|b| *b == b'9').unwrap();
    body[at] = b'8';
    f.http.put(&url, body);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Signature { .. }), "{err}");
    assert!(f.installed_binary().contains(OLD_SHA));
}

#[tokio::test]
async fn wrong_key_truncated_and_missing_signatures_are_refused() {
    let f = fixture_with_release();
    let sig_url = format!("{REPO_BASE}/v0.9.1/manifest.json.minisig");
    let body = f
        .http
        .get_raw(&format!("{REPO_BASE}/v0.9.1/manifest.json"))
        .unwrap();

    // signed by someone else, for both the pointer and the manifest
    let other = Signer::new();
    f.http.put(&sig_url, other.sign(&body));
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdateError::Signature { .. }),
        "wrong key: {err}"
    );

    let good = f.signer.sign(&body);
    f.http.put(&sig_url, good[..good.len() / 2].to_vec());
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdateError::Signature { .. }),
        "truncated: {err}"
    );

    f.http.remove(&sig_url);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdateError::Signature { .. }),
        "missing: {err}"
    );

    f.http.put(&sig_url, b"garbage".to_vec());
    assert!(matches!(
        f.engine()
            .apply(&ApplyOptions::default())
            .await
            .unwrap_err(),
        UpdateError::Signature { .. }
    ));

    f.http.put(&sig_url, good);
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
}

#[tokio::test]
async fn unsigned_channel_pointer_is_refused() {
    let f = fixture_with_release();
    f.http.remove(&format!(
        "{REPO_BASE}/channel-main/channel-main.json.minisig"
    ));
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Signature { .. }), "{err}");
}

#[tokio::test]
async fn pointer_must_match_its_manifest_and_stay_under_the_repo() {
    let f = fixture_with_release();
    let url = format!("{REPO_BASE}/channel-main/channel-main.json");
    let pointer = |manifest_url: &str, sha: &str| {
        serde_json::to_vec(&serde_json::json!({"schema":1,"channel":"main","version":"0.9.1","manifest_url":manifest_url,"manifest_sha256":sha})).unwrap()
    };
    f.http.put_signed(
        &f.signer,
        &url,
        pointer(
            &format!("{REPO_BASE}/v0.9.1/manifest.json"),
            &"0".repeat(64),
        ),
    );
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::ChecksumMismatch { .. }), "{err}");

    f.http.put_signed(
        &f.signer,
        &url,
        pointer("https://evil.test/manifest.json", &"0".repeat(64)),
    );
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Invalid { .. }), "{err}");
}

#[tokio::test]
async fn unsupported_schema_is_refused_even_when_signed() {
    let f = Fixture::new();
    let url = format!("{REPO_BASE}/v0.9.1/manifest.json");
    f.http.put_signed(
        &f.signer,
        &url,
        br#"{"schema":2,"version":"0.9.1"}"#.to_vec(),
    );
    let err = f
        .engine()
        .apply(&ApplyOptions {
            pin: Some("0.9.1".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdateError::UnsupportedSchema { found: 2, .. }),
        "{err}"
    );
}

#[tokio::test]
async fn checksum_mismatch_refuses_and_cleans_up() {
    let f = fixture_with_release();
    let url = format!("{REPO_BASE}/v0.9.1/hq-0.9.1-linux-x86_64.tar.gz");
    let mut bytes = f.http.get_raw(&url).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    f.http.put(&url, bytes);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::ChecksumMismatch { .. }), "{err}");
    assert!(f.installed_binary().contains(OLD_SHA));
    assert!(!Layout::from_config(&f.cfg).staged_binary().exists());
    assert!(
        f.world.events().is_empty(),
        "nothing ran: {:?}",
        f.world.events()
    );
}

#[tokio::test]
async fn size_limits_apply_before_and_after_download() {
    let mut f = fixture_with_release();
    f.cfg.limits.max_binary_bytes = 10;
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::SizeLimit { .. }), "{err}");

    // artifact bigger than the manifest promised
    let mut f = fixture_with_release();
    let url = format!("{REPO_BASE}/v0.9.1/hq-0.9.1-linux-x86_64.tar.gz");
    let mut bytes = f.http.get_raw(&url).unwrap();
    bytes.extend_from_slice(&[0u8; 64]);
    f.http.put(&url, bytes);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            UpdateError::SizeLimit { .. } | UpdateError::ChecksumMismatch { .. }
        ),
        "{err}"
    );
    f.cfg.limits.max_manifest_bytes = 5;
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::SizeLimit { .. }), "{err}");
    assert!(f.installed_binary().contains(OLD_SHA));
}

#[tokio::test]
async fn updater_too_old_is_refused() {
    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.min_updater = "0.10.0".into();
    f.publish_and_point(&spec);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::UpdaterTooOld { .. }), "{err}");
}

#[tokio::test]
async fn staged_binary_that_cannot_run_aborts_but_never_blocks_the_release() {
    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.binary = Some(b"BROKEN".to_vec());
    f.publish_and_point(&spec);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(!matches!(err, UpdateError::StagedBinary(_)), "{err}");
    assert!(f.installed_binary().contains(OLD_SHA));
    assert_eq!(f.web_index(), "web 0.9.0 index.html");
    assert!(
        f.world.events().is_empty(),
        "no snapshot, notice or restart: {:?}",
        f.world.events()
    );
    assert!(!Layout::from_config(&f.cfg).staged_binary().exists());
    assert!(!Layout::from_config(&f.cfg).staged_web().exists());
    // the service user could break exec on purpose, so this must not suppress later updates
    assert!(
        !State::load(&Layout::from_config(&f.cfg))
            .unwrap()
            .is_blocked("0.9.1")
    );
}

#[tokio::test]
async fn staged_binary_reporting_the_wrong_build_blocks_until_the_block_expires() {
    let mut f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.binary = Some(binary_content("0.9.1", "deadbeef999"));
    f.publish_and_point(&spec);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::StagedBinary(_)), "{err}");
    let again = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(again, Outcome::Skipped { .. }), "{again:?}");

    // blocks expire, so a failure the service user provoked cannot pin the host forever
    let layout = Layout::from_config(&f.cfg);
    let mut state = State::load(&layout).unwrap();
    state.blocked[0].until = 1;
    state.save(&layout).unwrap();
    f.publish_and_point(&ReleaseSpec::new("0.9.1", "deadbeef999"));
    f.cfg.blocked_ttl_secs = 3600;
    let outcome = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(applied(&outcome), "{outcome:?}");
}

#[tokio::test]
async fn web_archive_missing_files_or_escaping_is_refused() {
    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.web_items = Some(vec![("index.html".into(), b"x".to_vec())]);
    f.publish_and_point(&spec);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Invalid { .. }), "{err}");
    assert_eq!(f.web_index(), "web 0.9.0 index.html");

    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.web_items = Some(vec![("../../escape.txt".into(), b"x".to_vec())]);
    f.publish_and_point(&spec);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::UnsafeArchive(_)), "{err}");
    assert!(!f.dir.path().join("escape.txt").exists());
    assert!(f.installed_binary().contains(OLD_SHA));
}

#[tokio::test]
async fn release_without_web_artifact_only_swaps_the_binary() {
    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.with_web = false;
    f.publish_and_point(&spec);
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    assert_eq!(f.web_index(), "web 0.9.0 index.html");
    assert!(!Layout::from_config(&f.cfg).prev_web().exists());
}

#[tokio::test]
async fn health_failure_rolls_back_binary_web_and_database() {
    let f = fixture_with_release();
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    f.world
        .migrating_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    let outcome = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    let Outcome::RolledBack {
        attempted,
        restored,
        db_restored,
        ..
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        (attempted.as_str(), restored.as_str(), db_restored),
        ("0.9.1", "0.9.0", true)
    );
    assert!(f.installed_binary().contains(OLD_SHA));
    assert_eq!(f.web_index(), "web 0.9.0 index.html");
    assert_eq!(*f.world.db_content.lock().unwrap(), "db-v0");
    assert_eq!(
        *f.world.running_sha.lock().unwrap(),
        Some(OLD_SHA.to_string())
    );
    let events = f.world.events();
    assert!(events.contains(&"db-restore".to_string()), "{events:?}");
    assert!(
        events
            .iter()
            .any(|e| e.starts_with("notify:post-rolled-back"))
    );
    assert!(!events.iter().any(|e| e.starts_with("install --upgrade")));
    let layout = Layout::from_config(&f.cfg);
    let state = State::load(&layout).unwrap();
    assert!(state.history.is_empty());
    assert!(state.is_blocked("0.9.1"));

    // no retry loop: the timer skips the failed release
    let again = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(again, Outcome::Skipped { .. }), "{again:?}");

    // an explicit pin overrides the block
    f.world.unhealthy_shas.lock().unwrap().clear();
    let pinned = f
        .engine()
        .apply(&ApplyOptions {
            pin: Some("0.9.1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(applied(&pinned), "{pinned:?}");
}

#[tokio::test]
async fn database_is_left_alone_when_no_migration_ran() {
    let f = fixture_with_release();
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    let outcome = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::RolledBack {
                db_restored: false,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert!(!f.world.events().contains(&"db-restore".to_string()));
}

#[tokio::test]
async fn requires_db_snapshot_forces_a_database_restore() {
    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.requires_db_snapshot = true;
    f.publish_and_point(&spec);
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    let outcome = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::RolledBack {
                db_restored: true,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn failed_rollback_health_is_an_error() {
    let f = fixture_with_release();
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .extend([NEW_SHA.to_string(), OLD_SHA.to_string()]);
    assert!(f.engine().apply(&ApplyOptions::default()).await.is_err());
}

#[tokio::test]
async fn pin_allows_a_downgrade_and_pinned_current_is_a_noop() {
    let f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.8.0", "ccccccc3333"));
    let o = f
        .engine()
        .apply(&ApplyOptions {
            pin: Some("v0.8.0".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(applied(&o), "{o:?}");
    assert!(f.installed_binary().contains("ccccccc3333"));

    let o = f
        .engine_as("0.8.0", "ccccccc3333")
        .apply(&ApplyOptions {
            pin: Some("0.8.0".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(matches!(o, Outcome::UpToDate { .. }), "{o:?}");

    let mut cfg_pinned = Fixture::new();
    cfg_pinned.cfg.pin = Some("0.8.0".into());
    cfg_pinned.publish(&ReleaseSpec::new("0.8.0", "ccccccc3333"));
    assert!(applied(
        &cfg_pinned
            .engine()
            .apply(&ApplyOptions::default())
            .await
            .unwrap()
    ));
}

#[tokio::test]
async fn pin_must_name_the_version_the_manifest_carries() {
    let f = Fixture::new();
    let spec = ReleaseSpec::new("0.8.0", "ccccccc3333");
    f.publish(&spec);
    // a validly signed 0.8.0 manifest served at the 0.7.0 location
    let good = f
        .http
        .get_raw(&format!("{REPO_BASE}/v0.8.0/manifest.json"))
        .unwrap();
    f.http.put_signed(
        &f.signer,
        &format!("{REPO_BASE}/v0.7.0/manifest.json"),
        good,
    );
    let err = f
        .engine()
        .apply(&ApplyOptions {
            pin: Some("0.7.0".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Invalid { .. }), "{err}");
}

#[tokio::test]
async fn manual_rollback_restores_previous_and_blocks_current() {
    let f = fixture_with_release();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    *f.world.db_content.lock().unwrap() = "db-after-update".into();

    let after = f.engine_as("0.9.1", NEW_SHA);
    let outcome = after.rollback(false).await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::ManualRollback {
                db_restored: false,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert!(f.installed_binary().contains(OLD_SHA));
    assert_eq!(f.web_index(), "web 0.9.0 index.html");
    assert_eq!(*f.world.db_content.lock().unwrap(), "db-after-update");
    let state = State::load(&Layout::from_config(&f.cfg)).unwrap();
    assert!(state.is_blocked("0.9.1"));

    // the timer does not walk straight back into the version we left
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(o, Outcome::Skipped { .. }), "{o:?}");

    let err = f.engine().rollback(false).await.unwrap_err();
    assert!(matches!(err, UpdateError::NoRollbackTarget), "{err}");
}

#[tokio::test]
async fn manual_rollback_can_restore_the_database() {
    let f = fixture_with_release();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    *f.world.db_content.lock().unwrap() = "db-after-update".into();
    let outcome = f.engine_as("0.9.1", NEW_SHA).rollback(true).await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::ManualRollback {
                db_restored: true,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(*f.world.db_content.lock().unwrap(), "db-v0");
}

#[tokio::test]
async fn concurrent_update_is_refused_by_the_lock() {
    let f = fixture_with_release();
    let layout = Layout::from_config(&f.cfg);
    let _held = UpdateLock::acquire(&layout.lock_file()).unwrap();
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Locked), "{err}");
    let err = f.engine().rollback(false).await.unwrap_err();
    assert!(matches!(err, UpdateError::Locked), "{err}");
    assert!(f.installed_binary().contains(OLD_SHA));
    // read-only operations do not need the lock
    assert!(f.engine().check(None).await.unwrap().update_available);
}

#[tokio::test]
async fn successive_updates_keep_three_binaries_and_prune_snapshots() {
    let mut f = Fixture::new();
    f.cfg.snapshots_keep = 2;
    let mut current = ("0.9.0".to_string(), OLD_SHA.to_string());
    for i in 1..=5u32 {
        let (v, sha) = (format!("0.9.{i}"), format!("{i}{i}{i}{i}{i}{i}{i}a"));
        f.publish_and_point(&ReleaseSpec::new(&v, &sha));
        let o = f
            .engine_as(&current.0, &current.1)
            .apply(&ApplyOptions::default())
            .await
            .unwrap();
        assert!(applied(&o), "{o:?}");
        current = (v, sha);
    }
    let layout = Layout::from_config(&f.cfg);
    assert!(layout.backup(1).exists() && layout.backup(2).exists() && layout.backup(3).exists());
    assert!(!layout.backup(4).exists());
    assert!(
        fs::read_to_string(layout.backup(1))
            .unwrap()
            .contains("version=0.9.4")
    );
    // snapshots still referenced by a kept backup slot are never pruned
    assert_eq!(fs::read_dir(layout.snapshots_dir()).unwrap().count(), 4);
}

#[tokio::test]
async fn force_reinstalls_the_current_version() {
    let f = Fixture::new();
    f.publish_and_point(&ReleaseSpec::new("0.9.0", OLD_SHA));
    let o = f
        .engine()
        .apply(&ApplyOptions {
            force: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(applied(&o), "{o:?}");
    assert_eq!(f.web_index(), "web 0.9.0 index.html");
}

#[tokio::test]
async fn pointer_for_another_channel_is_refused() {
    let f = fixture_with_release();
    let manifest_url = format!("{REPO_BASE}/v0.9.1/manifest.json");
    let manifest = f.http.get_raw(&manifest_url).unwrap();
    let pointer = serde_json::to_vec(&serde_json::json!({
        "schema": 1, "channel": "stable", "version": "0.9.1",
        "manifest_url": manifest_url, "manifest_sha256": sha256_hex(&manifest),
    }))
    .unwrap();
    // a validly signed stable pointer served at the main pointer's URL
    f.http.put_signed(
        &f.signer,
        &format!("{REPO_BASE}/channel-main/channel-main.json"),
        pointer,
    );
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Invalid { .. }), "{err}");
    assert!(f.installed_binary().contains(OLD_SHA));
}

#[tokio::test]
async fn configured_pin_does_not_retry_a_blocked_release_but_cli_pin_does() {
    let mut f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.9.1", NEW_SHA));
    f.cfg.pin = Some("0.9.1".into());
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    let first = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(first, Outcome::RolledBack { .. }), "{first:?}");
    let restarts = f
        .world
        .events()
        .iter()
        .filter(|e| e.starts_with("restart"))
        .count();
    let second = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(second, Outcome::Skipped { .. }), "{second:?}");
    assert_eq!(
        f.world
            .events()
            .iter()
            .filter(|e| e.starts_with("restart"))
            .count(),
        restarts,
        "no restart on the skipped tick"
    );
    f.world.unhealthy_shas.lock().unwrap().clear();
    let forced = f
        .engine()
        .apply(&ApplyOptions {
            pin: Some("0.9.1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(applied(&forced), "{forced:?}");
}

#[tokio::test]
async fn check_honours_the_configured_pin() {
    let mut f = fixture_with_release();
    f.publish(&ReleaseSpec::new("0.8.0", "ccccccc3333"));
    f.cfg.pin = Some("0.9.0".into());
    f.publish(&ReleaseSpec::new("0.9.0", OLD_SHA));
    let report = f.engine().check(None).await.unwrap();
    assert_eq!(report.available.as_deref(), Some("0.9.0"));
    assert!(!report.update_available, "pinned to the running version");
}

#[tokio::test]
async fn failed_web_swap_restores_the_binary_and_does_not_restart() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return; // root ignores directory permissions
    }
    let f = fixture_with_release();
    let parent = f.cfg.web_dist.parent().unwrap().to_path_buf();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let err = f.engine().apply(&ApplyOptions::default()).await;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(err.is_err());
    assert!(f.installed_binary().contains(OLD_SHA), "binary put back");
    assert!(f.world.events().iter().all(|e| !e.starts_with("restart")));
    let layout = Layout::from_config(&f.cfg);
    assert!(
        State::load(&layout).unwrap().history.is_empty(),
        "backup slot undone with the swap"
    );
    assert!(!layout.backup(1).exists());
    assert!(!layout.staged_binary().exists());
}

#[tokio::test]
async fn manual_rollback_with_missing_snapshot_changes_nothing() {
    let f = fixture_with_release();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    let layout = Layout::from_config(&f.cfg);
    let state = State::load(&layout).unwrap();
    fs::remove_file(state.history[0].db_snapshot.as_ref().unwrap()).unwrap();
    let before = f.world.events().len();
    let err = f
        .engine_as("0.9.1", NEW_SHA)
        .rollback(true)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("snapshot"), "{err}");
    assert!(
        f.installed_binary().contains(NEW_SHA),
        "still on the new build"
    );
    assert_eq!(
        f.world.events().len(),
        before + 1,
        "only the pre notice, no stop or restart"
    );
}

#[tokio::test]
async fn leftover_work_dir_from_a_killed_run_is_swept() {
    let f = fixture_with_release();
    let layout = Layout::from_config(&f.cfg);
    let stale = layout.work_dir().join(std::process::id().to_string());
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("hq-0.9.1-linux-x86_64.tar.gz"), "partial").unwrap();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
}

#[test]
fn health_sha_comparison_never_panics_on_odd_input() {
    use crate::engine::sha_matches_for_tests as m;
    assert!(m("bbbbbbb2222", "bbbbbbb22221234"));
    assert!(!m("bbbbbbb2222", "ccccccc3333"));
    assert!(!m("unknown", "bbbbbbb2222"));
    assert!(!m("bbbbbbb\u{e9}22", "bbbbbbb2222"));
    assert!(!m("", ""));
}

async fn crash_during_apply(f: &Fixture) {
    use futures::FutureExt;
    *f.world.crash_on_restart.lock().unwrap() = true;
    let attempt = std::panic::AssertUnwindSafe(f.engine().apply(&ApplyOptions::default()))
        .catch_unwind()
        .await;
    assert!(attempt.is_err(), "the simulated crash must abort the run");
}

fn has_event(f: &Fixture, prefix: &str) -> bool {
    f.world.events().iter().any(|e| e.starts_with(prefix))
}

#[tokio::test]
async fn crash_after_swap_before_restart_is_finished_by_the_next_run() {
    let f = fixture_with_release();
    crash_during_apply(&f).await;
    assert!(f.installed_binary().contains(NEW_SHA), "swap happened");
    let layout = Layout::from_config(&f.cfg);
    assert!(
        State::load(&layout).unwrap().in_progress.is_some(),
        "journal left behind"
    );
    assert_eq!(
        *f.world.running_sha.lock().unwrap(),
        Some(OLD_SHA.to_string())
    );

    // the next timer tick runs the freshly installed binary
    let o = f
        .engine_as("0.9.1", NEW_SHA)
        .apply(&ApplyOptions::default())
        .await
        .unwrap();
    assert!(matches!(o, Outcome::UpToDate { .. }), "{o:?}");
    assert_eq!(
        *f.world.running_sha.lock().unwrap(),
        Some(NEW_SHA.to_string())
    );
    let state = State::load(&layout).unwrap();
    assert!(state.in_progress.is_none());
    assert_eq!(state.history.len(), 1, "backup slot is recorded");
    assert!(has_event(
        &f,
        "alert:an earlier update to 0.9.1 was interrupted"
    ));
}

#[tokio::test]
async fn crash_after_swap_with_unhealthy_build_is_rolled_back_by_the_next_run() {
    let f = fixture_with_release();
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    crash_during_apply(&f).await;
    let _ = f
        .engine_as("0.9.1", NEW_SHA)
        .apply(&ApplyOptions::default())
        .await;
    assert!(f.installed_binary().contains(OLD_SHA));
    assert_eq!(f.web_index(), "web 0.9.0 index.html");
    let state = State::load(&Layout::from_config(&f.cfg)).unwrap();
    assert!(state.in_progress.is_none() && state.history.is_empty());
    assert!(state.is_blocked("0.9.1"));
    assert!(has_event(&f, "alert:the interrupted update"));
}

#[tokio::test]
async fn crash_between_the_web_renames_is_repaired() {
    let f = fixture_with_release();
    crash_during_apply(&f).await;
    // live tree already moved aside, staged tree not yet renamed into place
    fs::rename(&f.cfg.web_dist, Layout::from_config(&f.cfg).staged_web()).unwrap();
    assert!(!f.cfg.web_dist.exists());
    let o = f
        .engine_as("0.9.1", NEW_SHA)
        .apply(&ApplyOptions::default())
        .await
        .unwrap();
    assert!(matches!(o, Outcome::UpToDate { .. }), "{o:?}");
    assert_eq!(f.web_index(), "web 0.9.1 index.html");
}

#[tokio::test]
async fn missing_live_web_tree_is_restored_from_prev_when_the_old_binary_is_still_installed() {
    let f = Fixture::new();
    let layout = Layout::from_config(&f.cfg);
    fs::rename(&f.cfg.web_dist, layout.prev_web()).unwrap();
    f.publish_and_point(&ReleaseSpec::new("0.9.0", OLD_SHA));
    f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert_eq!(f.web_index(), "web 0.9.0 index.html");
}

#[tokio::test]
async fn corrupt_state_file_is_quarantined_not_fatal() {
    let f = fixture_with_release();
    let layout = Layout::from_config(&f.cfg);
    fs::create_dir_all(&layout.state_dir).unwrap();
    fs::write(layout.state_file(), b"{ not json").unwrap();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    let aside: Vec<_> = fs::read_dir(&layout.state_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("state.json.corrupt-")
        })
        .collect();
    assert_eq!(aside.len(), 1);

    fs::write(layout.state_file(), b"").unwrap();
    assert_eq!(State::load(&layout).unwrap(), State::default());
}

#[tokio::test]
async fn snapshot_failure_is_a_warning_unless_the_release_requires_it() {
    let f = fixture_with_release();
    *f.world.snapshot_fails.lock().unwrap() = true;
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    let Outcome::Applied { warnings, .. } = o else {
        panic!("{o:?}")
    };
    assert_eq!(warnings.len(), 1);
    assert!(has_event(&f, "alert:database snapshot skipped"));
    assert!(f.installed_binary().contains(NEW_SHA));

    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.requires_db_snapshot = true;
    f.publish_and_point(&spec);
    *f.world.snapshot_fails.lock().unwrap() = true;
    assert!(f.engine().apply(&ApplyOptions::default()).await.is_err());
    assert!(f.installed_binary().contains(OLD_SHA));
    assert!(!has_event(&f, "restart"));
}

#[tokio::test]
async fn automatic_rollback_sends_a_loud_alert() {
    let f = fixture_with_release();
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(has_event(&f, "alert:update to 0.9.1 failed"));
}

#[tokio::test]
async fn failed_database_restore_still_restarts_and_keeps_state_in_step() {
    let f = fixture_with_release();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    *f.world.restore_fails.lock().unwrap() = true;
    let err = f
        .engine_as("0.9.1", NEW_SHA)
        .rollback(true)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("database"), "{err}");
    assert!(f.installed_binary().contains(OLD_SHA));
    assert_eq!(
        *f.world.running_sha.lock().unwrap(),
        Some(OLD_SHA.to_string()),
        "service back up"
    );
    let layout = Layout::from_config(&f.cfg);
    assert!(State::load(&layout).unwrap().history.is_empty());
    assert!(!layout.backup(1).exists());
}

#[tokio::test]
async fn failing_web_restore_does_not_desync_state_from_the_backup_files() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let f = fixture_with_release();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    let parent = f.cfg.web_dist.parent().unwrap().to_path_buf();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let err = f.engine_as("0.9.1", NEW_SHA).rollback(false).await;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(err.is_err());
    let layout = Layout::from_config(&f.cfg);
    assert!(f.installed_binary().contains(OLD_SHA));
    assert!(!layout.backup(1).exists(), "backup was consumed");
    assert!(
        State::load(&layout).unwrap().history.is_empty(),
        "and the state says so"
    );
    assert_eq!(
        *f.world.running_sha.lock().unwrap(),
        Some(OLD_SHA.to_string())
    );
}

#[tokio::test]
async fn unreadable_migration_count_skips_the_database_restore() {
    let f = fixture_with_release();
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    f.world
        .migrating_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    f.world
        .count_fails_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(
        matches!(
            o,
            Outcome::RolledBack {
                db_restored: false,
                ..
            }
        ),
        "{o:?}"
    );
    assert!(!has_event(&f, "db-restore"));
}

#[tokio::test]
async fn stale_process_does_not_act_after_another_updater_finished() {
    let f = fixture_with_release();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    // a second process that started before the swap still believes it is 0.9.0
    f.publish_and_point(&ReleaseSpec::new("0.9.2", "ddddddd4444"));
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(o, Outcome::Skipped { .. }), "{o:?}");
    assert!(f.installed_binary().contains(NEW_SHA));
    let state = State::load(&Layout::from_config(&f.cfg)).unwrap();
    assert_eq!(state.history.len(), 1);
    assert_eq!(
        state.history[0].installed.version, "0.9.0",
        "hq.1 is not mislabeled"
    );
}

#[tokio::test]
async fn manifest_urls_with_traversal_are_refused() {
    let f = fixture_with_release();
    let url = format!("{REPO_BASE}/channel-main/channel-main.json");
    for tail in [
        "v0.9.1/../v0.9.1/manifest.json",
        "v0.9.1/%2e%2e/v0.9.1/manifest.json",
        "v0.9.1//manifest.json",
    ] {
        let pointer = serde_json::to_vec(&serde_json::json!({
            "schema":1,"channel":"main","version":"0.9.1",
            "manifest_url": format!("{REPO_BASE}/{tail}"), "manifest_sha256": "0".repeat(64),
        }))
        .unwrap();
        f.http.put_signed(&f.signer, &url, pointer);
        let err = f
            .engine()
            .apply(&ApplyOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, UpdateError::Invalid { .. }), "{tail}: {err}");
    }
}

#[tokio::test]
async fn version_check_compares_tokens_exactly() {
    let f = Fixture::new();
    let mut spec = ReleaseSpec::new("0.9.1", NEW_SHA);
    spec.binary = Some(binary_content("0.9.10", NEW_SHA));
    f.publish_and_point(&spec);
    let err = f
        .engine()
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdateError::StagedBinary(_)),
        "0.9.10 must not satisfy 0.9.1: {err}"
    );
}

fn point_seq(f: &Fixture, version: &str, seq: Option<u64>, at: Option<&str>) {
    f.http
        .point_channel_with(&f.signer, "main", version, seq, at);
}

fn recorded_seq(f: &Fixture) -> Option<u64> {
    State::load(&Layout::from_config(&f.cfg))
        .unwrap()
        .seen_pointer("main")
        .map(|p| p.seq)
}

#[tokio::test]
async fn replayed_pointer_with_a_lower_seq_is_refused() {
    let f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.9.1", NEW_SHA));
    f.publish(&ReleaseSpec::new("0.9.2", "ddddddd4444"));
    point_seq(&f, "0.9.1", Some(2000), None);
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    assert_eq!(recorded_seq(&f), Some(2000));

    // an older, still validly signed pointer is replayed
    point_seq(&f, "0.9.2", Some(1000), None);
    let err = f
        .engine_as("0.9.1", NEW_SHA)
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, UpdateError::Invalid { .. }), "{err}");
    assert!(err.to_string().contains("replayed"), "{err}");
    assert!(
        f.engine_as("0.9.1", NEW_SHA).check(None).await.is_err(),
        "check refuses too"
    );
    assert!(f.installed_binary().contains(NEW_SHA));

    // a higher seq is fine
    point_seq(&f, "0.9.2", Some(3000), None);
    assert!(applied(
        &f.engine_as("0.9.1", NEW_SHA)
            .apply(&ApplyOptions::default())
            .await
            .unwrap()
    ));
    assert_eq!(recorded_seq(&f), Some(3000));
}

#[tokio::test]
async fn same_seq_with_different_content_is_refused_but_identical_is_fine() {
    let f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.9.1", NEW_SHA));
    f.publish(&ReleaseSpec::new("0.9.2", "ddddddd4444"));
    point_seq(&f, "0.9.1", Some(2000), None);
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    point_seq(&f, "0.9.2", Some(2000), None);
    let err = f
        .engine_as("0.9.1", NEW_SHA)
        .apply(&ApplyOptions::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("different content"), "{err}");
    point_seq(&f, "0.9.1", Some(2000), None);
    let o = f
        .engine_as("0.9.1", NEW_SHA)
        .apply(&ApplyOptions::default())
        .await
        .unwrap();
    assert!(matches!(o, Outcome::UpToDate { .. }), "{o:?}");
}

#[tokio::test]
async fn pointer_without_seq_is_accepted_and_records_nothing() {
    let f = fixture_with_release();
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    assert_eq!(recorded_seq(&f), None);
}

#[tokio::test]
async fn seq_is_recorded_only_after_success_or_a_higher_noop() {
    let f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.9.1", NEW_SHA));
    point_seq(&f, "0.9.1", Some(2000), None);
    f.world
        .unhealthy_shas
        .lock()
        .unwrap()
        .insert(NEW_SHA.into());
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(o, Outcome::RolledBack { .. }), "{o:?}");
    assert_eq!(recorded_seq(&f), None, "a failed update records nothing");
    let d = f
        .engine()
        .apply(&ApplyOptions {
            dry_run: true,
            ..Default::default()
        })
        .await;
    assert!(d.is_ok());
    assert_eq!(recorded_seq(&f), None, "dry runs record nothing");

    // channel head equals the installed version: up to date, but the higher seq is remembered
    let g = Fixture::new();
    g.publish_and_point(&ReleaseSpec::new("0.9.0", OLD_SHA));
    point_seq(&g, "0.9.0", Some(4000), None);
    let o = g.engine().apply(&ApplyOptions::default()).await.unwrap();
    assert!(matches!(o, Outcome::UpToDate { .. }), "{o:?}");
    assert_eq!(recorded_seq(&g), Some(4000));
}

#[tokio::test]
async fn pin_bypasses_replay_protection() {
    let f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.9.1", NEW_SHA));
    f.publish(&ReleaseSpec::new("0.8.0", "ccccccc3333"));
    point_seq(&f, "0.9.1", Some(2000), None);
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    let o = f
        .engine_as("0.9.1", NEW_SHA)
        .apply(&ApplyOptions {
            pin: Some("0.8.0".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(applied(&o), "{o:?}");
    assert_eq!(recorded_seq(&f), Some(2000), "a pin never lowers the mark");
}

#[tokio::test]
async fn lost_state_forgets_the_high_water_mark() {
    let f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.9.1", NEW_SHA));
    f.publish(&ReleaseSpec::new("0.9.2", "ddddddd4444"));
    point_seq(&f, "0.9.1", Some(2000), None);
    assert!(applied(
        &f.engine().apply(&ApplyOptions::default()).await.unwrap()
    ));
    let layout = Layout::from_config(&f.cfg);
    fs::write(layout.state_file(), b"garbage").unwrap();
    point_seq(&f, "0.9.2", Some(1000), None);
    // documented limit: with state gone the mark is gone; versions still only move forward
    assert!(applied(
        &f.engine_as("0.9.1", NEW_SHA)
            .apply(&ApplyOptions::default())
            .await
            .unwrap()
    ));
    assert_eq!(recorded_seq(&f), Some(1000));
}

#[tokio::test]
async fn stale_pointer_is_flagged_only_when_max_age_is_configured() {
    let mut f = Fixture::new();
    f.publish(&ReleaseSpec::new("0.9.1", NEW_SHA));
    point_seq(&f, "0.9.1", Some(2000), Some("2020-01-01T00:00:00Z"));
    let quiet = f.engine().check(None).await.unwrap();
    assert!(quiet.note.is_none(), "off by default: {:?}", quiet.note);

    f.cfg.max_pointer_age_secs = Some(3600);
    let report = f.engine().check(None).await.unwrap();
    assert!(report.note.unwrap().contains("stale or frozen"));
    let o = f.engine().apply(&ApplyOptions::default()).await.unwrap();
    let Outcome::Applied { warnings, .. } = o else {
        panic!("{o:?}")
    };
    assert!(
        warnings.iter().any(|w| w.contains("stale or frozen")),
        "{warnings:?}"
    );
}

// ---- integration: real reqwest client against a local HTTP server ----

mod http_integration {
    use super::*;
    use crate::engine::Engine;
    use crate::real::ReqwestHttp;
    use axum::Router;
    use axum::extract::State as AxState;
    use axum::http::{StatusCode, Uri};
    use axum::response::IntoResponse;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    async fn handler(AxState(files): AxState<Files>, uri: Uri) -> impl IntoResponse {
        match files.lock().unwrap().get(uri.path()) {
            Some(body) => (StatusCode::OK, body.clone()).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    /// Starts a server on a free port and returns its origin plus the file map to fill.
    async fn start_server() -> (String, Files) {
        let files: Files = Arc::default();
        let app = Router::new().fallback(handler).with_state(files.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (origin, files)
    }

    fn mirror(f: &Fixture, origin: &str, files: &Files) {
        let mut map = files.lock().unwrap();
        map.clear();
        for (url, body) in f.http.files.lock().unwrap().iter() {
            map.insert(url.strip_prefix(origin).unwrap().to_string(), body.clone());
        }
    }

    fn engine<'a>(f: &'a Fixture, http: &'a ReqwestHttp) -> Engine<'a> {
        Engine {
            cfg: &f.cfg,
            key: f.signer.public_key(),
            updater_version: semver::Version::parse("0.9.0").unwrap(),
            current: crate::state::Installed {
                version: "0.9.0".into(),
                git_sha: OLD_SHA.into(),
            },
            layout: Layout::from_config(&f.cfg),
            http,
            restarter: &f.world,
            health: &f.world,
            host: &f.world,
        }
    }

    #[tokio::test]
    async fn full_update_over_real_http() {
        let (origin, files) = start_server().await;
        let f = Fixture::with_origin(&origin);
        f.publish_and_point(&ReleaseSpec::new("0.9.1", NEW_SHA));
        mirror(&f, &origin, &files);

        let http = ReqwestHttp::new(true).unwrap();
        let outcome = engine(&f, &http)
            .apply(&ApplyOptions::default())
            .await
            .unwrap();
        assert!(applied(&outcome), "{outcome:?}");
        assert!(f.installed_binary().contains(NEW_SHA));
        assert_eq!(f.web_index(), "web 0.9.1 index.html");
    }

    #[tokio::test]
    async fn real_http_rejects_404_oversize_and_tampering() {
        let (origin, files) = start_server().await;
        let mut f = Fixture::with_origin(&origin);
        f.publish_and_point(&ReleaseSpec::new("0.9.1", NEW_SHA));
        mirror(&f, &origin, &files);
        let http = ReqwestHttp::new(true).unwrap();

        // oversize: content-length above the manifest cap is refused before reading
        f.cfg.limits.max_manifest_bytes = 8;
        let err = engine(&f, &http)
            .apply(&ApplyOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, UpdateError::SizeLimit { .. }), "{err}");
        f.cfg.limits.max_manifest_bytes = 1 << 20;

        // tampered artifact served over HTTP
        let key = "/acme/hq/releases/download/v0.9.1/hq-0.9.1-linux-x86_64.tar.gz";
        files.lock().unwrap().get_mut(key).unwrap().push(0);
        let err = engine(&f, &http)
            .apply(&ApplyOptions::default())
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                UpdateError::SizeLimit { .. } | UpdateError::ChecksumMismatch { .. }
            ),
            "{err}"
        );

        // missing signature file
        files
            .lock()
            .unwrap()
            .remove("/acme/hq/releases/download/channel-main/channel-main.json.minisig");
        let err = engine(&f, &http)
            .apply(&ApplyOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, UpdateError::Signature { .. }), "{err}");
        assert!(f.installed_binary().contains(OLD_SHA));
    }
}
