//! End to end: real vaults, real databases (created through `hq_db`, so with the real migrated
//! schema), real files on disk. Argon2 runs at a tiny cost here so the suite stays fast; the
//! default cost is covered by its own bounds test.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hq_backup::{
    Argon2Cost, BackupError, BackupOptions, RestoreOptions, Secret, create, restore, verify,
};
use rusqlite::Connection;

const PASSPHRASE: &str = "correct horse battery staple";
const TEST_COST: Argon2Cost = Argon2Cost {
    memory_kib: 64,
    passes: 1,
    lanes: 1,
};
const PROBE_ROWS: i64 = 500;

struct Fixture {
    dir: tempfile::TempDir,
    vault: PathBuf,
    db: PathBuf,
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    write(
        &vault.join("Notebooks/Projects/Alpha/plan.md"),
        "# Plan\n\nBody text.\n",
    );
    write(
        &vault.join("Notebooks/Projects/Alpha/Notas ñ 日本.md"),
        "unicode name\n",
    );
    write(&vault.join("Notebooks/Inbox.md"), &"line\n".repeat(50_000));
    write(&vault.join("_system/SOUL.md"), "soul\n");
    // A path past the 100 bytes a plain tar name holds, so the long-name record is exercised.
    write(
        &vault.join(format!(
            "Notebooks/Projects/{}/{}/a-note-with-a-long-path.md",
            "p".repeat(60),
            "q".repeat(60)
        )),
        "long path\n",
    );
    for empty in ["Notebooks/Inbox", "_mailboxes/relay", "_agents"] {
        fs::create_dir_all(vault.join(empty)).unwrap();
    }
    // Real note titles have colons in them, and a backslash is a legal name on Unix.
    #[cfg(unix)]
    {
        write(
            &vault.join("Notebooks/Projects/Sunbird AI Meeting: Agent-HQ Uganda-Local.md"),
            "colon\n",
        );
        write(
            &vault.join("Notebooks/Projects/back\\slash.md"),
            "backslash\n",
        );
    }
    write(&vault.join("_data/archive/old.txt"), "kept\n");
    write(&vault.join("_logs/daemon.log"), "never backed up\n");
    write(&vault.join("_data/.consolidate-lock"), "");
    write(
        &vault.join("_data/vault.db.bak-20260101"),
        "old on-box copy",
    );
    // HQ's own locks name a process that will not exist after a restore. A `.lock` file that is
    // a person's own (a lockfile in a project note folder, or any name HQ does not use) is data.
    write(&vault.join("_sync/sync.lock"), "");
    write(&vault.join(".discord-relay/bot.lock"), "");
    write(&vault.join("_system/.daemon-lock"), "");
    write(&vault.join(".pairing-telegram.lock"), "");
    write(
        &vault.join("Notebooks/Projects/Alpha/Cargo.lock"),
        "lockfile\n",
    );
    write(&vault.join("Notebooks/Projects/Alpha/bot.lock"), "mine\n");
    write(&vault.join("notes.lock"), "lock");
    let db = vault.join("_data/vault.db");
    drop(hq_db::Database::open(&db).unwrap());
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE backup_probe (id INTEGER PRIMARY KEY, body TEXT);")
        .unwrap();
    for i in 0..PROBE_ROWS {
        conn.execute(
            "INSERT INTO backup_probe (body) VALUES (?1)",
            [format!("row {i}")],
        )
        .unwrap();
    }
    drop(conn);
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hostname", vault.join("link.md")).unwrap();
    Fixture { dir, vault, db }
}

fn options<'a>(f: &'a Fixture, out: &'a Path, secret: &'a Secret) -> BackupOptions<'a> {
    BackupOptions {
        vault: &f.vault,
        db: &f.db,
        config_file: None,
        out,
        secret,
        hq_version: "test",
        cost: TEST_COST,
        verify: true,
    }
}

fn restore_opts<'a>(archive: &'a Path, into: &'a Path, secret: &'a Secret) -> RestoreOptions<'a> {
    RestoreOptions {
        archive,
        into,
        secret,
        config_out: None,
        reindex: false,
        max_bytes: None,
    }
}

fn passphrase() -> Secret {
    Secret::passphrase(PASSPHRASE).unwrap()
}

fn key_secret(dir: &Path) -> Secret {
    let path = dir.join("backup.key");
    Secret::generate_key_file(&path).unwrap();
    Secret::key_file(&path).unwrap()
}

fn probe_rows(db: &Path) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM backup_probe", [], |r| r.get(0))
        .unwrap()
}

fn integrity(db: &Path) -> String {
    Connection::open(db)
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap()
}

/// HQ's own lock files, outside the notes, which a backup leaves out.
fn hq_lock(rel: &str, name: &str) -> bool {
    let lock = [".consolidate-lock", ".daemon-lock", "bot.lock", "sync.lock"].contains(&name)
        || (name.starts_with(".pairing-") && name.ends_with(".lock"));
    lock && !rel.starts_with("Notebooks/")
}

/// Every plain file under `root` that a backup is meant to carry, by relative path.
fn notes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                if rel != "_logs" {
                    walk(root, &path, out);
                }
            } else if kind.is_file() {
                let name = entry.file_name().to_str().unwrap().to_string();
                let sidecar = rel.starts_with("_data/vault.db");
                if !sidecar && !hq_lock(&rel, &name) {
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn leftovers(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_string())
        .filter(|n| {
            n.starts_with(".hq-restore-") || n.ends_with(".partial") || n.ends_with(".work")
        })
        .collect()
}

#[test]
fn a_backup_restores_to_the_same_notes_and_database_with_a_passphrase_or_a_key_file() {
    let f = fixture();
    for (label, secret) in [
        ("passphrase", passphrase()),
        ("key file", key_secret(f.dir.path())),
    ] {
        let out = f
            .dir
            .path()
            .join(format!("{}.hqbak", label.replace(' ', "-")));
        let report = create(&options(&f, &out, &secret)).unwrap();
        assert!(report.verified && report.archive_bytes > 0, "{label}");
        assert!(
            report.skipped.iter().any(|s| s.contains("link.md")),
            "the link is reported, not followed: {:?}",
            report.skipped
        );

        let into = f
            .dir
            .path()
            .join(format!("restored-{}", label.replace(' ', "-")));
        let restored = restore(&restore_opts(&out, &into, &secret)).unwrap();
        assert_eq!(restored.vault, into);
        assert!(restored.warnings.is_empty(), "{:?}", restored.warnings);

        assert_eq!(notes(&f.vault), notes(&into), "{label}: notes differ");
        for empty in ["Notebooks/Inbox", "_mailboxes/relay", "_agents"] {
            assert!(
                into.join(empty).is_dir(),
                "{label}: empty folder {empty} was not restored"
            );
        }
        assert!(!into.join("_logs").exists());
        assert_eq!(
            probe_rows(&into.join("_data/vault.db")),
            PROBE_ROWS,
            "{label}"
        );
        assert_eq!(integrity(&into.join("_data/vault.db")), "ok");

        // What must not travel did not.
        assert!(!into.join("_logs").exists());
        for lock in [
            "_sync/sync.lock",
            ".discord-relay/bot.lock",
            "_system/.daemon-lock",
            ".pairing-telegram.lock",
            "_data/.consolidate-lock",
        ] {
            assert!(!into.join(lock).exists(), "{lock}");
        }
        for mine in [
            "Notebooks/Projects/Alpha/Cargo.lock",
            "Notebooks/Projects/Alpha/bot.lock",
            "notes.lock",
        ] {
            assert!(into.join(mine).is_file(), "{mine} is data, not an HQ lock");
        }
        assert!(!into.join("_data/vault.db.bak-20260101").exists());
        assert!(!into.join("link.md").exists());
        let paths: Vec<_> = restored
            .manifest
            .files
            .iter()
            .map(|e| e.path.as_str())
            .collect();
        assert!(
            paths
                .iter()
                .all(|p| !p.contains("_logs") && !p.contains("sync.lock")),
            "{paths:?}"
        );
        assert!(paths.contains(&"vault/Notebooks/Projects/Alpha/Notas ñ 日本.md"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&into), 0o700);
            assert_eq!(mode(&into.join("Notebooks/Inbox.md")), 0o600);
            assert_eq!(mode(&into.join("_data/vault.db")), 0o600);
            assert_eq!(mode(&out), 0o600, "the archive itself is private");
        }
        assert!(
            leftovers(f.dir.path()).is_empty(),
            "{:?}",
            leftovers(f.dir.path())
        );
    }
}

#[test]
fn the_backup_is_encrypted_and_does_not_show_what_it_holds() {
    let f = fixture();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &passphrase())).unwrap();
    let bytes = fs::read(&out).unwrap();
    // The payload after the 54-byte header is sealed, not a gzip stream that merely hides text:
    // a gzip stream starts 1f 8b, and a second backup of the same vault shares no payload bytes.
    const HEADER_LEN: usize = 54;
    assert_ne!(&bytes[HEADER_LEN..HEADER_LEN + 2], &[0x1f, 0x8b]);
    let again = f.dir.path().join("b2.hqbak");
    create(&options(&f, &again, &passphrase())).unwrap();
    let other = fs::read(&again).unwrap();
    let same = bytes[HEADER_LEN..]
        .iter()
        .zip(&other[HEADER_LEN..])
        .filter(|(a, b)| a == b)
        .count();
    assert!(
        same < bytes.len() / 50,
        "two backups of one vault share {same} payload bytes"
    );
    for needle in [
        "# Plan",
        "Body text",
        "Notas",
        "Notebooks",
        "SQLite format",
        "backup_probe",
        "row 17",
    ] {
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "{needle} is visible in the backup file"
        );
    }
}

#[test]
fn a_wrong_passphrase_or_key_is_refused_and_nothing_is_written() {
    let f = fixture();
    let right = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &right)).unwrap();
    let into = f.dir.path().join("restored");

    let wrong = Secret::passphrase("a different long passphrase").unwrap();
    assert!(matches!(
        restore(&restore_opts(&out, &into, &wrong)),
        Err(BackupError::Decrypt)
    ));
    // A key file where a passphrase was used, and the other way round, says so.
    let key = key_secret(f.dir.path());
    assert!(matches!(
        restore(&restore_opts(&out, &into, &key)),
        Err(BackupError::Refused(m)) if m.contains("passphrase")
    ));
    let keyed = f.dir.path().join("keyed.hqbak");
    create(&options(&f, &keyed, &key)).unwrap();
    assert!(matches!(
        restore(&restore_opts(&keyed, &into, &right)),
        Err(BackupError::Refused(m)) if m.contains("key file")
    ));
    let other_key = {
        let p = f.dir.path().join("other.key");
        Secret::generate_key_file(&p).unwrap();
        Secret::key_file(&p).unwrap()
    };
    assert!(matches!(
        restore(&restore_opts(&keyed, &into, &other_key)),
        Err(BackupError::Decrypt)
    ));
    assert!(!into.exists());
    assert!(leftovers(f.dir.path()).is_empty());
}

#[test]
fn every_kind_of_damage_is_refused_and_nothing_is_written() {
    let f = fixture();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();
    let good = fs::read(&out).unwrap();
    let last = good.len() - 1;
    let flips = [
        0,
        8,
        9,
        12,
        30,
        60,
        100,
        good.len() / 3,
        good.len() / 2,
        last - 5,
        last,
    ];
    let cuts = [0, 20, 53, 54, 70, good.len() / 2, last];
    let into = f.dir.path().join("restored");

    for at in flips {
        let mut bad = good.clone();
        bad[at] ^= 0x04;
        let path = f.dir.path().join("flipped.hqbak");
        fs::write(&path, &bad).unwrap();
        assert!(
            restore(&restore_opts(&path, &into, &secret)).is_err(),
            "a flipped bit at byte {at} was accepted"
        );
        assert!(verify(&path, &secret).is_err(), "verify accepted byte {at}");
        assert!(
            !into.exists() && leftovers(f.dir.path()).is_empty(),
            "byte {at}"
        );
        fs::remove_file(&path).unwrap();
    }
    for cut in cuts {
        let path = f.dir.path().join("cut.hqbak");
        fs::write(&path, &good[..cut]).unwrap();
        assert!(
            restore(&restore_opts(&path, &into, &secret)).is_err(),
            "a backup cut to {cut} bytes was accepted"
        );
        assert!(
            !into.exists() && leftovers(f.dir.path()).is_empty(),
            "cut {cut}"
        );
        fs::remove_file(&path).unwrap();
    }
    // Appended junk is refused too.
    let mut longer = good.clone();
    longer.extend_from_slice(b"junk");
    let path = f.dir.path().join("longer.hqbak");
    fs::write(&path, longer).unwrap();
    assert!(restore(&restore_opts(&path, &into, &secret)).is_err());
    assert!(!into.exists());
}

#[test]
fn a_backup_with_a_database_from_a_newer_hq_is_refused_before_anything_is_written() {
    let f = fixture();
    Connection::open(&f.db)
        .unwrap()
        .execute(
            "INSERT INTO schema_version (version) VALUES ('999_from_the_future')",
            [],
        )
        .unwrap();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();
    let into = f.dir.path().join("restored");
    let err = restore(&restore_opts(&out, &into, &secret)).unwrap_err();
    assert!(
        matches!(&err, BackupError::Newer(m) if m.contains("999_from_the_future")),
        "{err}"
    );
    assert!(matches!(verify(&out, &secret), Err(BackupError::Newer(_))));
    assert!(!into.exists() && leftovers(f.dir.path()).is_empty());
}

#[test]
fn a_database_carrying_retired_migrations_restores() {
    // Real databases keep the rows of migrations that were later removed from the code.
    let f = fixture();
    Connection::open(&f.db)
        .unwrap()
        .execute(
            "INSERT INTO schema_version (version) VALUES ('006_model_cards')",
            [],
        )
        .unwrap();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();
    let into = f.dir.path().join("restored");
    restore(&restore_opts(&out, &into, &secret)).unwrap();
    assert_eq!(probe_rows(&into.join("_data/vault.db")), PROBE_ROWS);
}

#[test]
fn restoring_into_a_folder_that_is_not_empty_is_refused_and_an_empty_one_works() {
    let f = fixture();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();

    let busy = f.dir.path().join("busy");
    write(&busy.join("mine.md"), "my own file");
    let err = restore(&restore_opts(&out, &busy, &secret)).unwrap_err();
    assert!(matches!(err, BackupError::Refused(_)), "{err}");
    assert_eq!(
        fs::read_to_string(busy.join("mine.md")).unwrap(),
        "my own file"
    );
    assert!(!busy.join("Notebooks").exists());

    let file = f.dir.path().join("a-file");
    fs::write(&file, "x").unwrap();
    assert!(restore(&restore_opts(&out, &file, &secret)).is_err());

    let empty = f.dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    restore(&restore_opts(&out, &empty, &secret)).unwrap();
    assert_eq!(notes(&f.vault), notes(&empty));
    assert!(leftovers(&empty).is_empty(), "{:?}", leftovers(&empty));
    assert_eq!(probe_rows(&empty.join("_data/vault.db")), PROBE_ROWS);

    let nested = f.dir.path().join("a/b/new");
    restore(&restore_opts(&out, &nested, &secret)).unwrap();
    assert_eq!(probe_rows(&nested.join("_data/vault.db")), PROBE_ROWS);
}

#[test]
fn a_backup_taken_while_the_vault_is_being_written_restores_cleanly() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = fixture();
    let secret = passphrase();
    let stop = Arc::new(AtomicBool::new(false));
    // There from the start, so a backup that finds it missing is not a race with the writer.
    fs::write(f.vault.join("Notebooks/live.md"), "revision 0\n").unwrap();
    let writer = {
        let (db, vault, stop) = (f.db.clone(), f.vault.clone(), Arc::clone(&stop));
        std::thread::spawn(move || {
            let conn = Connection::open(&db).unwrap();
            conn.busy_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let mut i = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let _ = conn.execute(
                    "INSERT INTO backup_probe (body) VALUES (?1)",
                    [format!("live {i}")],
                );
                let _ = fs::write(vault.join("Notebooks/live.md"), format!("revision {i}\n"));
                // Files and folders that come and go while the backup walks the vault.
                let (a, b) = (
                    vault.join("Notebooks/churn-a.md"),
                    vault.join("Notebooks/churn-b.md"),
                );
                let _ = fs::write(&a, format!("churn {i}\n"));
                let _ = fs::rename(&a, &b);
                let _ = fs::remove_file(&b);
                let folder = vault.join(format!("Notebooks/churn-folder-{}", i % 4));
                let _ = fs::create_dir(&folder);
                let _ = fs::write(folder.join("x.md"), "x");
                let _ = fs::remove_dir_all(&folder);
                i += 1;
            }
        })
    };
    for round in 0..3 {
        let out = f.dir.path().join(format!("live-{round}.hqbak"));
        let report = create(&options(&f, &out, &secret));
        let report = report.unwrap_or_else(|e| panic!("round {round}: {e}"));
        assert!(report.verified);
        let into = f.dir.path().join(format!("live-restored-{round}"));
        restore(&restore_opts(&out, &into, &secret)).unwrap();
        assert_eq!(
            integrity(&into.join("_data/vault.db")),
            "ok",
            "round {round}"
        );
        assert!(
            probe_rows(&into.join("_data/vault.db")) >= PROBE_ROWS,
            "round {round}"
        );
        // A note being rewritten is either there whole, from one revision, or left out and said so.
        match fs::read_to_string(into.join("Notebooks/live.md")) {
            Ok(text) => assert!(
                text.starts_with("revision ")
                    && text.ends_with('\n')
                    && text.matches('\n').count() == 1,
                "round {round}: a torn note was backed up: {text:?}"
            ),
            Err(_) => assert!(
                report.skipped.iter().any(|s| s.contains("live.md")),
                "round {round}: the note is missing and nothing says why: {:?}",
                report.skipped
            ),
        }
        // Everything that was not being written is exactly as it was.
        assert_eq!(
            fs::read(into.join("Notebooks/Projects/Alpha/plan.md")).unwrap(),
            fs::read(f.vault.join("Notebooks/Projects/Alpha/plan.md")).unwrap()
        );
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
}

#[test]
fn the_config_is_left_out_unless_asked_for_and_restored_only_where_told() {
    let f = fixture();
    let secret = passphrase();
    let config = f.dir.path().join("config.yaml");
    write(&config, "openrouter_api_key: sk-test-not-a-real-key\n");

    let plain = f.dir.path().join("plain.hqbak");
    let report = create(&options(&f, &plain, &secret)).unwrap();
    assert!(report.skipped.iter().all(|s| !s.contains("config")));
    assert!(verify(&plain, &secret).unwrap().config.is_none());

    let with = f.dir.path().join("with.hqbak");
    let mut opts = options(&f, &with, &secret);
    opts.config_file = Some(&config);
    create(&opts).unwrap();
    assert!(
        fs::read(&with)
            .unwrap()
            .windows(10)
            .all(|w| w != b"sk-test-no")
    );

    let into = f.dir.path().join("r1");
    let no_path = restore(&restore_opts(&with, &into, &secret)).unwrap();
    assert!(no_path.config_restored.is_none());
    assert!(
        no_path.warnings.iter().any(|w| w.contains("config")),
        "{:?}",
        no_path.warnings
    );
    assert!(!into.join("config.yaml").exists());
    assert!(!f.dir.path().join("config.restored").exists());

    let out_path = f.dir.path().join("restored-config.yaml");
    let into = f.dir.path().join("r2");
    let mut r = restore_opts(&with, &into, &secret);
    r.config_out = Some(&out_path);
    let done = restore(&r).unwrap();
    assert_eq!(done.config_restored.as_deref(), Some(out_path.as_path()));
    assert_eq!(
        fs::read_to_string(&out_path).unwrap(),
        "openrouter_api_key: sk-test-not-a-real-key\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&out_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // A config path that cannot be written is a warning: the vault is restored, not undone.
    let blocker = f.dir.path().join("blocker");
    fs::write(&blocker, "a file, not a folder").unwrap();
    let bad_out = blocker.join("config.yaml");
    let into = f.dir.path().join("r4");
    let mut r = restore_opts(&with, &into, &secret);
    r.config_out = Some(&bad_out);
    let done = restore(&r).unwrap();
    assert!(done.config_restored.is_none());
    assert!(
        done.warnings
            .iter()
            .any(|w| w.contains("could not be written")),
        "{:?}",
        done.warnings
    );
    assert_eq!(probe_rows(&into.join("_data/vault.db")), PROBE_ROWS);

    // It never replaces a file that is already there.
    let into = f.dir.path().join("r3");
    let mut r = restore_opts(&with, &into, &secret);
    r.config_out = Some(&out_path);
    assert!(restore(&r).is_err());
    assert!(!into.exists());
}

#[test]
fn a_backup_is_never_overwritten_never_goes_into_the_vault_and_a_folder_gets_a_timestamped_name() {
    let f = fixture();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();
    assert!(
        create(&options(&f, &out, &secret)).is_err(),
        "an existing backup was overwritten"
    );

    let inside = f.vault.join("Notebooks");
    let err = create(&options(&f, &inside.join("b.hqbak"), &secret)).unwrap_err();
    assert!(matches!(err, BackupError::Refused(m) if m.contains("outside the vault")));
    assert!(!inside.join("b.hqbak").exists());

    let folder = f.dir.path().join("backups");
    fs::create_dir(&folder).unwrap();
    let report = create(&options(&f, &folder, &secret)).unwrap();
    let name = report.path.file_name().unwrap().to_str().unwrap();
    assert!(hq_backup::is_backup_name(name), "{name}");
    assert_eq!(hq_backup::list(&folder).unwrap(), vec![report.path]);
}

#[test]
fn two_backups_in_the_same_second_both_survive_and_still_sort_by_time() {
    let f = fixture();
    let secret = passphrase();
    let folder = f.dir.path().join("backups");
    fs::create_dir(&folder).unwrap();
    let first = create(&options(&f, &folder, &secret)).unwrap().path;
    let second = create(&options(&f, &folder, &secret)).unwrap().path;
    assert_ne!(first, second);
    assert!(first.exists() && second.exists());
    assert!(
        first.file_name() < second.file_name(),
        "{first:?} {second:?}"
    );
    assert_eq!(hq_backup::list(&folder).unwrap().len(), 2);
}

fn entries_in(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_failed_backup_leaves_nothing_behind_whenever_it_fails() {
    let f = fixture();
    let secret = passphrase();
    let outdir = f.dir.path().join("out");
    fs::create_dir(&outdir).unwrap();
    let out = outdir.join("b.hqbak");

    // Fails before any work: no database, or a config that is not there.
    let missing = f.dir.path().join("no-such");
    let mut opts = options(&f, &out, &secret);
    opts.db = &missing;
    assert!(create(&opts).is_err());
    let mut opts = options(&f, &out, &secret);
    opts.config_file = Some(&missing);
    assert!(create(&opts).is_err());

    // Fails after the output file and the work folder exist: a "database" that is not one.
    let junk = f.dir.path().join("junk.db");
    fs::write(&junk, vec![7u8; 4096]).unwrap();
    let mut opts = options(&f, &out, &secret);
    opts.db = &junk;
    assert!(create(&opts).is_err());

    assert!(entries_in(&outdir).is_empty(), "{:?}", entries_in(&outdir));
}

#[test]
fn a_file_that_is_already_there_is_never_mistaken_for_ours_and_removed() {
    let f = fixture();
    let secret = passphrase();
    let outdir = f.dir.path().join("out");
    fs::create_dir(&outdir).unwrap();
    let out = outdir.join("b.hqbak");
    let theirs = outdir.join("b.hqbak.partial");
    fs::write(&theirs, "another backup is being written here").unwrap();

    let err = create(&options(&f, &out, &secret)).unwrap_err();
    assert!(matches!(err, BackupError::Refused(_)), "{err}");
    assert_eq!(
        fs::read_to_string(&theirs).unwrap(),
        "another backup is being written here"
    );
    assert!(!out.exists());
}

#[test]
fn verify_reports_what_a_backup_holds_without_writing_anything() {
    let f = fixture();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    let report = create(&options(&f, &out, &secret)).unwrap();
    let before = leftovers(f.dir.path());
    assert!(matches!(
        hq_backup::verify_limited(&out, &secret, Some(10_000)),
        Err(BackupError::Refused(m)) if m.contains("allowed size")
    ));
    let manifest = verify(&out, &secret).unwrap();
    assert_eq!(manifest.file_count(), report.files);
    assert_eq!(manifest.total_bytes(), report.bytes);
    assert_eq!(manifest.hq_version, "test");
    assert_eq!(leftovers(f.dir.path()), before);
}

#[test]
fn a_restore_larger_than_the_allowed_size_is_refused() {
    let f = fixture();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();
    let into = f.dir.path().join("restored");
    let mut r = restore_opts(&out, &into, &secret);
    r.max_bytes = Some(10_000);
    assert!(matches!(restore(&r), Err(BackupError::Refused(m)) if m.contains("allowed size")));
    assert!(!into.exists() && leftovers(f.dir.path()).is_empty());
}

fn indexed_notes(db: &Path) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM notes_fts", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn restoring_can_rebuild_the_search_index() {
    let f = fixture();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();

    let without = f.dir.path().join("without");
    restore(&restore_opts(&out, &without, &secret)).unwrap();
    assert_eq!(
        indexed_notes(&without.join("_data/vault.db")),
        0,
        "the fixture database has never indexed a note, so any rows come from the rebuild"
    );

    let into = f.dir.path().join("restored");
    let mut r = restore_opts(&out, &into, &secret);
    r.reindex = true;
    let done = restore(&r).unwrap();
    assert!(done.warnings.is_empty(), "{:?}", done.warnings);
    assert_eq!(integrity(&into.join("_data/vault.db")), "ok");
    assert!(
        indexed_notes(&into.join("_data/vault.db")) >= 3,
        "the rebuild indexed nothing"
    );
}

#[test]
fn a_file_bigger_than_the_in_memory_limit_is_staged_on_disk_and_restores_exactly() {
    let f = fixture();
    let secret = passphrase();
    let big: Vec<u8> = (0..9 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let path = f.vault.join("Notebooks/big.bin");
    fs::write(&path, &big).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();

    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();
    let into = f.dir.path().join("restored");
    restore(&restore_opts(&out, &into, &secret)).unwrap();
    assert!(fs::read(into.join("Notebooks/big.bin")).unwrap() == big);
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_read_is_left_out_and_listed_not_a_reason_to_fail() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let secret = passphrase();
    let locked = f.vault.join("Notebooks/locked.md");
    fs::write(&locked, "private").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&locked).is_ok() {
        return; // running as a user that can read anything: nothing to prove here
    }
    let hidden_dir = f.vault.join("Notebooks/closed");
    fs::create_dir(&hidden_dir).unwrap();
    fs::write(hidden_dir.join("inside.md"), "x").unwrap();
    fs::set_permissions(&hidden_dir, fs::Permissions::from_mode(0o000)).unwrap();

    let out = f.dir.path().join("b.hqbak");
    let report = create(&options(&f, &out, &secret));
    // Put the modes back so the temporary folder can be removed.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&hidden_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let report = report.unwrap();
    assert_eq!(report.unreadable, 2, "{:?}", report.skipped);
    let said = report.skipped.join("\n");
    assert!(
        said.contains("locked.md") && said.contains("not readable"),
        "{said}"
    );
    assert!(said.contains("closed"), "{said}");
    assert!(verify(&out, &secret).is_ok());
}

#[test]
fn secrets_are_checked_before_they_are_used() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Secret::passphrase("short").is_err());
    assert!(Secret::passphrase("").is_err());
    assert!(Secret::passphrase("exactly ten").is_ok());

    let key = dir.path().join("k.key");
    Secret::generate_key_file(&key).unwrap();
    assert!(
        Secret::generate_key_file(&key).is_err(),
        "a key file must never be replaced"
    );
    assert!(Secret::key_file(&key).is_ok());
    assert!(
        !format!("{:?}", Secret::key_file(&key).unwrap())
            .contains(fs::read_to_string(&key).unwrap().trim())
    );

    let junk = dir.path().join("junk.key");
    fs::write(&junk, "not a key\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&junk, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(Secret::key_file(&junk).is_err());
    assert!(Secret::key_file(&dir.path().join("missing.key")).is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        let err = Secret::key_file(&key).unwrap_err();
        assert!(err.to_string().contains("chmod 600"), "{err}");
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link.key");
        std::os::unix::fs::symlink(&key, &link).unwrap();
        assert!(
            Secret::key_file(&link).is_err(),
            "a key file may not be a link"
        );
    }
}

#[test]
fn a_folder_that_does_not_exist_yet_is_made_and_a_name_ending_hqbak_is_a_file() {
    let f = fixture();
    let secret = passphrase();
    let folder = f.dir.path().join("fresh").join("backups");
    let report = create(&options(&f, &folder, &secret)).unwrap();
    assert!(folder.is_dir(), "a name with no .hqbak is a folder to make");
    assert_eq!(report.path.parent(), Some(folder.as_path()));
    assert!(hq_backup::is_backup_name(
        report.path.file_name().unwrap().to_str().unwrap()
    ));
    assert_eq!(hq_backup::list(&folder).unwrap(), vec![report.path]);

    let file = f.dir.path().join("elsewhere").join("mine.hqbak");
    let report = create(&options(&f, &file, &secret)).unwrap();
    assert_eq!(report.path, file);
    assert!(file.is_file());
}

#[test]
fn a_path_deeper_than_a_restore_accepts_is_left_out_and_said_so_not_made_into_a_backup_that_fails()
{
    let f = fixture();
    let secret = passphrase();
    let mut deep = f.vault.join("Notebooks");
    for _ in 0..70 {
        deep.push("d");
    }
    write(&deep.join("too-deep.md"), "deep\n");
    let out = f.dir.path().join("b.hqbak");
    let report = create(&options(&f, &out, &secret)).unwrap();
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.contains("too long for a backup")),
        "{:?}",
        report.skipped
    );
    let into = f.dir.path().join("restored");
    restore(&restore_opts(&out, &into, &secret)).unwrap();
    assert_eq!(notes(&f.vault).len() - 1, notes(&into).len());
    assert!(
        into.join("Notebooks/Projects")
            .join("p".repeat(60))
            .join("q".repeat(60))
            .join("a-note-with-a-long-path.md")
            .is_file(),
        "a long but allowed path still travels"
    );
}

#[test]
fn what_an_interrupted_restore_left_is_named_when_fresh_and_cleared_when_old() {
    let f = fixture();
    let secret = passphrase();
    let out = f.dir.path().join("b.hqbak");
    create(&options(&f, &out, &secret)).unwrap();
    let into = f.dir.path().join("target");
    fs::create_dir(&into).unwrap();
    let left = into.join(".hq-restore-aabbccddeeff");
    fs::create_dir(&left).unwrap();
    fs::write(left.join("plaintext"), "x").unwrap();

    let err = restore(&restore_opts(&out, &into, &secret)).unwrap_err();
    assert!(err.to_string().contains("interrupted"), "{err}");
    assert!(
        left.exists(),
        "a recent one may belong to a restore still running"
    );

    let two_days = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 3600);
    fs::File::open(&left)
        .unwrap()
        .set_modified(two_days)
        .unwrap();
    restore(&restore_opts(&out, &into, &secret)).unwrap();
    assert!(!left.exists() && into.join("Notebooks").is_dir());
}

#[cfg(unix)]
#[test]
fn a_fifo_where_a_key_file_should_be_is_refused_not_waited_on() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("key.fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: `name` is a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(Secret::key_file(&fifo).is_err());
}
