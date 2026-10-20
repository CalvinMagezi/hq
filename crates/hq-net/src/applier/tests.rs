//! Tests for the opener and the run loop. They need no root: the real `wg` is replaced by a
//! recorder, and files are made in a temporary directory owned by the test user.

use super::*;
use crate::key;
use std::cell::RefCell;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

fn k(seed: u8) -> String {
    key::encode(&[seed; 32])
}

fn valid_text(peers: u8) -> String {
    let mut text = format!("{}\n", grammar::HEADER);
    for i in 0..peers {
        text.push_str(&format!(
            "\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.{}/32\n",
            k(i + 1),
            i + 2
        ));
    }
    text.push_str(&format!("{}{peers}\n", grammar::TRAILER_PREFIX));
    text
}

fn write(path: &Path, bytes: &[u8], mode: u32) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn config(input: PathBuf) -> Config {
    parse_args(
        [
            "--interface",
            "hq0",
            "--subnet",
            "10.77.0.0/24",
            "--hub",
            "10.77.0.1",
            "--owner-uid",
            &me().to_string(),
            "--input",
            input.to_str().unwrap(),
        ]
        .map(String::from),
    )
    .unwrap()
}

#[derive(Default)]
struct Recorder {
    calls: RefCell<Vec<(String, String)>>,
    shown: RefCell<usize>,
    fail_apply: bool,
    /// What `wg showconf` prints; the default has one old peer the file never lists.
    show_text: Option<String>,
    /// Runs inside `apply`, to stand in for the input changing while the applier works.
    during_apply: RefCell<Option<Box<dyn FnMut()>>>,
    order: Option<Order>,
}

/// A firewall that must never be asked: a version 1 file says nothing about forwarding.
struct NoFirewall;

impl Firewall for NoFirewall {
    fn list_links(&self, _: &str) -> Result<String, String> {
        panic!("a version 1 file reached the firewall")
    }

    fn set_links(&self, _: &str, _: &str) -> Result<(), String> {
        panic!("a version 1 file reached the firewall")
    }
}

type Order = Rc<RefCell<Vec<&'static str>>>;

/// Records the scripts it is given, and in what order relative to `wg` when both share an `Order`.
#[derive(Default)]
struct FwRecorder {
    scripts: RefCell<Vec<String>>,
    /// What `nft list set` prints; `None` makes it fail.
    listing: Option<String>,
    fail_set: bool,
    /// Fails the first call only, to stand in for a firewall that was briefly unavailable.
    fail_first: std::cell::Cell<bool>,
    order: Option<Order>,
}

impl Firewall for FwRecorder {
    fn list_links(&self, _: &str) -> Result<String, String> {
        self.listing
            .clone()
            .ok_or_else(|| "no such set".to_string())
    }

    fn set_links(&self, _: &str, script: &str) -> Result<(), String> {
        if let Some(order) = &self.order {
            order.borrow_mut().push("nft");
        }
        self.scripts.borrow_mut().push(script.to_string());
        if self.fail_set || self.fail_first.replace(false) {
            Err("nft failed".into())
        } else {
            Ok(())
        }
    }
}

const SHOWCONF_KEY_SEED: u8 = 99;

impl Wg for Recorder {
    fn show_config(&self, _interface: &str) -> Result<String, String> {
        *self.shown.borrow_mut() += 1;
        if let Some(text) = &self.show_text {
            return Ok(text.clone());
        }
        Ok(format!(
            "[Interface]\nListenPort = 51820\nPrivateKey = {}\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.200/32\n",
            k(SHOWCONF_KEY_SEED),
            k(98)
        ))
    }

    fn apply(
        &self,
        _interface: &str,
        subcommand: wgconf::Subcommand,
        config: &str,
    ) -> Result<(), String> {
        if let Some(order) = &self.order {
            order.borrow_mut().push("wg");
        }
        self.calls
            .borrow_mut()
            .push((subcommand.arg().to_string(), config.to_string()));
        if let Some(hook) = self.during_apply.borrow_mut().as_mut() {
            hook();
        }
        if self.fail_apply {
            Err("wg failed".into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_private_plain_file_is_read_back_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(2).as_bytes(), 0o600);
    assert_eq!(read_input(&path, me()).unwrap(), valid_text(2).as_bytes());
}

#[test]
fn a_linked_final_component_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.conf");
    write(&real, valid_text(1).as_bytes(), 0o600);
    let link = dir.path().join("peers.conf");
    symlink(&real, &link).unwrap();
    assert!(matches!(read_input(&link, me()), Err(InputError::Link)));
}

#[test]
fn a_linked_directory_in_the_path_is_refused_even_when_the_file_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let real_dir = dir.path().join("real");
    std::fs::create_dir(&real_dir).unwrap();
    std::fs::set_permissions(&real_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    write(
        &real_dir.join("peers.conf"),
        valid_text(1).as_bytes(),
        0o600,
    );
    let link_dir = dir.path().join("spool");
    symlink(&real_dir, &link_dir).unwrap();
    assert!(matches!(
        read_input(&link_dir.join("peers.conf"), me()),
        Err(InputError::Link)
    ));
    assert!(read_input(&real_dir.join("peers.conf"), me()).is_ok());
}

#[test]
fn a_fifo_is_refused_without_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `c_path` is a valid NUL-terminated string for the whole call.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let (tx, rx) = mpsc::channel();
    let uid = me();
    std::thread::spawn(move || tx.send(read_input(&path, uid)).unwrap());
    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("opening a FIFO must not block");
    assert!(matches!(result, Err(InputError::NotRegular)));
}

#[test]
fn a_directory_and_a_missing_file_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("peers.conf");
    std::fs::create_dir(&sub).unwrap();
    assert!(matches!(
        read_input(&sub, me()),
        Err(InputError::NotRegular)
    ));
    assert!(matches!(
        read_input(&dir.path().join("none"), me()),
        Err(InputError::Io(_))
    ));
}

#[test]
fn a_second_name_for_the_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    std::fs::hard_link(&path, dir.path().join("other")).unwrap();
    assert!(matches!(
        read_input(&path, me()),
        Err(InputError::OtherNames)
    ));
}

#[test]
fn another_owner_is_refused_at_the_open_file_not_only_at_the_directory() {
    // Through the real opener the directory rule fires first (the test user cannot make a file
    // that someone else owns inside a directory it owns), so this covers that path...
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    assert!(matches!(
        read_input(&path, me() + 1),
        Err(InputError::ParentUnsafe)
    ));
    // ...and the file rule itself is checked on the facts an open file reports.
    let facts = |f: &dyn Fn(&mut Facts)| {
        let mut facts = Facts {
            is_file: true,
            uid: 1000,
            mode: 0o100600,
            nlink: 1,
            len: 100,
        };
        f(&mut facts);
        facts
    };
    assert!(check_file_facts(&facts(&|_| {}), 1000).is_ok());
    assert!(matches!(
        check_file_facts(&facts(&|_| {}), 1001),
        Err(InputError::WrongOwner)
    ));
    assert!(
        matches!(
            check_file_facts(&facts(&|f| f.uid = 0), 1000),
            Err(InputError::WrongOwner)
        ),
        "root-owned is not the expected owner either"
    );
    assert!(matches!(
        check_file_facts(&facts(&|f| f.is_file = false), 1000),
        Err(InputError::NotRegular)
    ));
    assert!(matches!(
        check_file_facts(&facts(&|f| f.mode = 0o100640), 1000),
        Err(InputError::TooOpen)
    ));
    assert!(matches!(
        check_file_facts(&facts(&|f| f.mode = 0o100604), 1000),
        Err(InputError::TooOpen)
    ));
    assert!(matches!(
        check_file_facts(&facts(&|f| f.nlink = 2), 1000),
        Err(InputError::OtherNames)
    ));
    assert!(matches!(
        check_file_facts(&facts(&|f| f.len = MAX_INPUT_BYTES as u64 + 1), 1000),
        Err(InputError::TooLarge)
    ));
    assert!(check_file_facts(&facts(&|f| f.len = MAX_INPUT_BYTES as u64), 1000).is_ok());
}

#[test]
fn the_directory_rule_takes_root_or_the_owner_and_nobody_else() {
    let dir = |uid: u32, mode: u32| Facts {
        is_file: false,
        uid,
        mode,
        nlink: 2,
        len: 0,
    };
    assert!(check_parent_facts(&dir(0, 0o040755), 1000).is_ok());
    assert!(check_parent_facts(&dir(1000, 0o040700), 1000).is_ok());
    assert!(matches!(
        check_parent_facts(&dir(1001, 0o040700), 1000),
        Err(InputError::ParentUnsafe)
    ));
    assert!(matches!(
        check_parent_facts(&dir(1000, 0o040770), 1000),
        Err(InputError::ParentUnsafe)
    ));
    assert!(matches!(
        check_parent_facts(&dir(1000, 0o040707), 1000),
        Err(InputError::ParentUnsafe)
    ));
    assert!(
        matches!(
            check_parent_facts(&dir(0, 0o041777), 1000),
            Err(InputError::ParentUnsafe)
        ),
        "a sticky world-writable directory such as /tmp"
    );
}

#[test]
fn group_or_other_access_is_refused() {
    for mode in [0o640, 0o604, 0o660, 0o644, 0o666, 0o700 | 0o010] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.conf");
        write(&path, valid_text(1).as_bytes(), mode);
        assert!(
            matches!(read_input(&path, me()), Err(InputError::TooOpen)),
            "{mode:o}"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o400);
    assert!(read_input(&path, me()).is_ok());
}

#[test]
fn a_directory_others_can_write_to_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    for mode in [0o770, 0o707, 0o777] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(
            matches!(read_input(&path, me()), Err(InputError::ParentUnsafe)),
            "{mode:o}"
        );
    }
}

#[test]
fn a_huge_file_is_refused_without_being_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, b"", 0o600);
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(MAX_INPUT_BYTES as u64 + 1).unwrap();
    assert!(matches!(read_input(&path, me()), Err(InputError::TooLarge)));
    file.set_len(100 * 1024 * 1024 * 1024).unwrap();
    assert!(matches!(read_input(&path, me()), Err(InputError::TooLarge)));
}

#[test]
fn relative_and_dotted_paths_are_refused() {
    for path in ["peers.conf", "./peers.conf", "/tmp/../etc/hostname", "/"] {
        assert!(
            matches!(read_input(Path::new(path), me()), Err(InputError::Path(_))),
            "{path}"
        );
    }
}

#[test]
fn a_valid_file_is_applied_with_the_live_interface_carried_forward() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(2).as_bytes(), 0o600);
    let wg = Recorder::default();
    assert_eq!(
        run(&config(path), &wg, &NoFirewall, &mut Vec::new())
            .unwrap()
            .len(),
        2
    );
    let calls = wg.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "syncconf");
    let text = &calls[0].1;
    assert!(text.starts_with(&format!(
        "[Interface]\nPrivateKey = {}\nListenPort = 51820\n",
        k(SHOWCONF_KEY_SEED)
    )));
    assert_eq!(text.matches("[Peer]").count(), 2);
    assert!(
        !text.contains("10.77.0.200"),
        "a peer the file no longer lists must not be carried over"
    );
}

#[test]
fn an_empty_peer_list_uses_setconf_so_the_last_peer_really_goes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(0).as_bytes(), 0o600);
    let wg = Recorder::default();
    assert_eq!(
        run(&config(path), &wg, &NoFirewall, &mut Vec::new())
            .unwrap()
            .len(),
        0
    );
    assert_eq!(wg.calls.borrow()[0].0, "setconf");
}

#[test]
fn a_refused_file_never_starts_wg_at_all() {
    let hostile = [
        format!("{}\n[Interface]\nPrivateKey = {}\n", grammar::HEADER, k(1)),
        format!(
            "{}\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.2/32\nPostUp = touch /tmp/x\n",
            grammar::HEADER,
            k(1)
        ),
        format!(
            "{}\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0\n",
            grammar::HEADER,
            k(1)
        ),
        String::new(),
        "not a peers file\n".to_string(),
    ];
    for text in hostile {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.conf");
        write(&path, text.as_bytes(), 0o600);
        let wg = Recorder::default();
        let err = run(&config(path), &wg, &NoFirewall, &mut Vec::new()).unwrap_err();
        assert!(matches!(err, ApplyError::Rejected(_)), "{err}");
        assert!(wg.calls.borrow().is_empty());
        assert_eq!(
            *wg.shown.borrow(),
            0,
            "not even showconf may run for a refused file"
        );
        assert!(!err.to_string().contains("PostUp") && !err.to_string().contains("touch"));
    }
}

#[test]
fn a_failing_wg_is_reported_with_what_it_was_trying_to_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(2).as_bytes(), 0o600);
    let wg = Recorder {
        fail_apply: true,
        ..Recorder::default()
    };
    let (result, log) = run_logged(path, &wg);
    assert!(matches!(result, Err(ApplyError::Wg(_))));
    assert!(
        log.contains("wg failed while putting 2 peer(s) on hq0"),
        "{log}"
    );
    assert!(
        log.contains(&format!("attempted added 10.77.0.2 {}", &k(1)[..8])),
        "{log}"
    );
    assert!(log.contains("attempted removed 10.77.0.200/32"), "{log}");
    assert!(
        !log.contains("peer(s) on hq0 after this run"),
        "a failed run is not recorded as applied: {log}"
    );
}

#[test]
fn unreadable_input_never_reaches_wg() {
    let dir = tempfile::tempdir().unwrap();
    let wg = Recorder::default();
    let missing = dir.path().join("missing.conf");
    assert!(matches!(
        run(&config(missing), &wg, &NoFirewall, &mut Vec::new()),
        Err(ApplyError::Input(_))
    ));
    assert!(wg.calls.borrow().is_empty());
}

fn args(list: &[&str]) -> Result<Config, String> {
    parse_args(list.iter().map(|s| s.to_string()))
}

const GOOD_ARGS: [&str; 10] = [
    "--interface",
    "hq0",
    "--subnet",
    "10.77.0.0/24",
    "--hub",
    "10.77.0.1",
    "--owner-uid",
    "1000",
    "--input",
    "/var/lib/hq-net/hq0.conf",
];

#[test]
fn the_units_arguments_are_taken_strictly() {
    let config = args(&GOOD_ARGS).unwrap();
    assert_eq!(config.interface, "hq0");
    assert_eq!(config.owner_uid, 1000);
    assert!(config.wg.is_none());
    let mut with_wg = GOOD_ARGS.to_vec();
    with_wg.extend(["--wg", "/usr/bin/wg"]);
    assert_eq!(
        args(&with_wg).unwrap().wg,
        Some(PathBuf::from("/usr/bin/wg"))
    );

    let swap = |flag: &str, value: &str| {
        let mut list = GOOD_ARGS.to_vec();
        let at = list.iter().position(|f| *f == flag).unwrap();
        list[at + 1] = value;
        args(&list)
    };
    for (flag, bad) in [
        ("--interface", "Hq0"),
        ("--interface", "0hq"),
        ("--interface", "hq-0"),
        ("--interface", "abcdefghijklmnop"),
        ("--interface", ""),
        ("--subnet", "8.8.8.0/24"),
        ("--subnet", "10.77.0.5/24"),
        ("--hub", "10.77.0.0"),
        ("--hub", "10.78.0.1"),
        ("--hub", "010.77.0.1"),
        ("--owner-uid", "-1"),
        ("--owner-uid", "root"),
    ] {
        assert!(swap(flag, bad).is_err(), "{flag} {bad}");
    }
    let mut twice = GOOD_ARGS.to_vec();
    twice.extend(["--hub", "10.77.0.2"]);
    assert!(args(&twice).is_err());
    let mut extra = GOOD_ARGS.to_vec();
    extra.push("--force");
    assert!(args(&extra).is_err());
    assert!(args(&GOOD_ARGS[..8]).is_err());
    let mut equals = GOOD_ARGS.to_vec();
    equals[1] = "--interface=hq0";
    assert!(args(&equals).is_err());
}

#[test]
fn a_path_to_wg_that_others_can_replace_is_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("wg");
    write(&fake, b"#!/bin/sh\n", 0o755);
    // Owned by the test user, not root, so it must be refused whatever its mode.
    if me() != ROOT_UID {
        assert!(trusted_binary(&fake).is_err());
    }
    assert!(trusted_binary(Path::new("relative/wg")).is_err());
    assert!(trusted_binary(&dir.path().join("none")).is_err());
}

#[test]
fn a_change_made_while_applying_is_applied_in_the_same_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    let wg = Recorder::default();
    let target = path.clone();
    let mut rewrote = false;
    *wg.during_apply.borrow_mut() = Some(Box::new(move || {
        if !rewrote {
            rewrote = true;
            std::fs::remove_file(&target).unwrap();
            write(&target, valid_text(3).as_bytes(), 0o600);
        }
    }));
    assert_eq!(
        run(&config(path), &wg, &NoFirewall, &mut Vec::new())
            .unwrap()
            .len(),
        3
    );
    let calls = wg.calls.borrow();
    assert_eq!(calls.len(), 2, "the second round applies the new file");
    assert_eq!(calls[1].1.matches("[Peer]").count(), 3);
}

#[test]
fn an_input_that_keeps_changing_is_applied_a_bounded_number_of_times() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    let wg = Recorder::default();
    let target = path.clone();
    let mut next = 2u8;
    *wg.during_apply.borrow_mut() = Some(Box::new(move || {
        std::fs::remove_file(&target).unwrap();
        write(&target, valid_text(next).as_bytes(), 0o600);
        next += 1;
    }));
    assert!(run(&config(path), &wg, &NoFirewall, &mut Vec::new()).is_ok());
    assert_eq!(wg.calls.borrow().len(), MAX_ROUNDS);
}

#[test]
fn a_file_replaced_by_something_hostile_after_the_first_apply_is_refused_and_the_first_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    let wg = Recorder::default();
    let target = path.clone();
    *wg.during_apply.borrow_mut() = Some(Box::new(move || {
        std::fs::remove_file(&target).unwrap();
        write(&target, b"[Interface]\nPostUp = id\n", 0o600);
    }));
    let (result, log) = run_logged(path, &wg);
    let err = result.unwrap_err();
    assert!(
        matches!(&err, ApplyError::AfterApply { applied: 1, cause } if matches!(**cause, ApplyError::Rejected(_))),
        "{err}"
    );
    assert!(!err.to_string().contains("PostUp"));
    assert_eq!(
        wg.calls.borrow().len(),
        1,
        "only the valid first version reached wg"
    );
    assert!(
        log.contains(&format!("added 10.77.0.2 {}", &k(1)[..8])),
        "{log}"
    );
}

fn run_logged(path: PathBuf, wg: &Recorder) -> (Result<Vec<grammar::Peer>, ApplyError>, String) {
    let mut out = Vec::new();
    let result = run(&config(path), wg, &NoFirewall, &mut out);
    (result, String::from_utf8(out).unwrap())
}

#[test]
fn the_journal_lists_what_a_run_changed_by_address_and_short_fingerprint_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(2).as_bytes(), 0o600);
    let wg = Recorder::default();
    let (result, log) = run_logged(path, &wg);
    assert_eq!(result.unwrap().len(), 2);
    assert!(log.contains("2 peer(s) on hq0 after this run"), "{log}");
    assert!(
        log.contains(&format!("added 10.77.0.2 {}", &k(1)[..8])),
        "{log}"
    );
    assert!(
        log.contains("removed 10.77.0.200/32"),
        "the peer the file no longer lists: {log}"
    );
    assert!(!log.contains(&k(1)), "only short fingerprints are written");
}

#[test]
fn a_run_that_changes_nothing_writes_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(0).as_bytes(), 0o600);
    let wg = Recorder {
        shown: RefCell::new(0),
        show_text: Some("[Interface]\nListenPort = 1\n".to_string()),
        ..Recorder::default()
    };
    let (result, log) = run_logged(path, &wg);
    assert_eq!(result.unwrap().len(), 0);
    assert_eq!(log, "hq-net-apply: no change, 0 peer(s) on hq0\n");
}

#[test]
fn a_peer_that_went_live_is_on_record_even_if_the_next_round_is_refused() {
    // The blocker case: root applies a peer, the writer then garbles the file before the re-read.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(2).as_bytes(), 0o600);
    let wg = Recorder::default();
    let target = path.clone();
    *wg.during_apply.borrow_mut() = Some(Box::new(move || {
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o666)).unwrap();
    }));
    let (result, log) = run_logged(path, &wg);
    let err = result.unwrap_err();
    assert!(
        matches!(&err, ApplyError::AfterApply { applied: 2, .. }),
        "{err}"
    );
    assert!(
        err.to_string()
            .contains("2 peer(s) had already been applied"),
        "{err}"
    );
    assert!(
        log.contains(&format!("added 10.77.0.2 {}", &k(1)[..8])),
        "audit lines missing: {log}"
    );
    assert!(
        log.contains(&format!("added 10.77.0.3 {}", &k(2)[..8])),
        "audit lines missing: {log}"
    );
    assert_eq!(wg.calls.borrow().len(), 1);
}

fn script_wg(dir: &Path, body: &str) -> SystemWg {
    let path = dir.join("wg");
    write(&path, format!("#!/bin/sh\n{body}\n").as_bytes(), 0o700);
    SystemWg { path }
}

#[test]
fn the_real_spawn_path_feeds_the_config_over_a_pipe_to_proc_self_fd_0() {
    // A stand-in program: it must be given `syncconf <interface> /proc/self/fd/0` and the text on stdin.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("received");
    let wg = script_wg(
        dir.path(),
        &format!(
            "[ \"$1\" = syncconf ] && [ \"$2\" = hq0 ] && [ \"$3\" = /proc/self/fd/0 ] || exit 3\ncat /proc/self/fd/0 > {}",
            out.display()
        ),
    );
    let text = "[Interface]\nListenPort = 1\n\n[Peer]\nPublicKey = x\n";
    wg.apply("hq0", wgconf::Subcommand::Syncconf, text).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), text);
    assert!(
        wg.apply("hq0", wgconf::Subcommand::Setconf, text).is_err(),
        "the stand-in rejects setconf"
    );
}

#[test]
fn the_real_spawn_path_reads_showconf_and_runs_with_an_empty_environment() {
    let dir = tempfile::tempdir().unwrap();
    let env_out = dir.path().join("env");
    let wg = script_wg(
        dir.path(),
        &format!(
            "[ \"$1\" = showconf ] && [ \"$2\" = hq0 ] || exit 3\n/usr/bin/env > {} 2>/dev/null\nprintf '[Interface]\\nListenPort = 51820\\n'",
            env_out.display()
        ),
    );
    assert_eq!(
        wg.show_config("hq0").unwrap(),
        "[Interface]\nListenPort = 51820\n"
    );
    let env = std::fs::read_to_string(&env_out).unwrap_or_default();
    assert!(
        !env.contains("HOME=") && !env.contains("CARGO"),
        "the environment must be empty: {env}"
    );
}

#[test]
fn a_failing_wg_is_an_error_not_a_success() {
    let dir = tempfile::tempdir().unwrap();
    let wg = script_wg(dir.path(), "cat > /dev/null; exit 1");
    assert!(
        wg.apply("hq0", wgconf::Subcommand::Syncconf, "x\n")
            .is_err()
    );
    assert!(wg.show_config("hq0").is_err());
}

#[test]
#[ignore = "needs root and the wireguard kernel module: sudo -E cargo test -p hq-net -- --ignored"]
fn the_real_wg_applies_a_peer_to_a_real_interface() {
    // Opt-in only. Creates and deletes an interface named hqnettest0, so it needs root and
    // wireguard support in the kernel, and it must not run on a machine whose network matters.
    assert_eq!(me(), ROOT_UID, "run this as root");
    let wg = SystemWg::locate(None).expect("wg must be installed");
    let ip = |args: &[&str]| {
        Command::new("ip")
            .args(args)
            .status()
            .expect("ip")
            .success()
    };
    assert!(ip(&["link", "add", "hqnettest0", "type", "wireguard"]));
    let outcome = std::panic::catch_unwind(|| {
        let shown = wg.show_config("hqnettest0").unwrap();
        let (sub, text) = wgconf::build(
            &[grammar::Peer {
                public_key: k(1),
                address: std::net::Ipv4Addr::new(10, 77, 0, 2),
                preshared_key: None,
            }],
            &shown,
        )
        .unwrap();
        wg.apply("hqnettest0", sub, &text).unwrap();
        assert!(wg.show_config("hqnettest0").unwrap().contains(&k(1)));
        let (sub, text) = wgconf::build(&[], &shown).unwrap();
        wg.apply("hqnettest0", sub, &text).unwrap();
        assert!(!wg.show_config("hqnettest0").unwrap().contains("[Peer]"));
    });
    assert!(ip(&["link", "del", "hqnettest0"]));
    outcome.unwrap();
}

fn valid_text_v2(peers: u8, links: &[(u8, u8)]) -> String {
    let mut text = format!("{}\n", grammar::HEADER_V2);
    for i in 0..peers {
        text.push_str(&format!(
            "\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.{}/32\n",
            k(i + 1),
            i + 2
        ));
    }
    for (a, b) in links {
        text.push_str(&format!(
            "\n[Link]\nA = 10.77.0.{a}/32\nB = 10.77.0.{b}/32\n"
        ));
    }
    text.push_str(&format!(
        "{}{peers} links {}\n",
        grammar::TRAILER_PREFIX,
        links.len()
    ));
    text
}

fn run_with_firewall(
    path: PathBuf,
    wg: &Recorder,
    fw: &FwRecorder,
) -> (Result<Vec<grammar::Peer>, ApplyError>, String) {
    let mut out = Vec::new();
    let result = run(&config(path), wg, fw, &mut out);
    (result, String::from_utf8(out).unwrap())
}

fn file_in(dir: &tempfile::TempDir, text: &str) -> PathBuf {
    let path = dir.path().join("peers.conf");
    write(&path, text.as_bytes(), 0o600);
    path
}

#[test]
fn a_version_2_file_sets_the_pairs_before_it_applies_the_peers() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(3, &[(2, 4)]));
    let order: Order = Rc::default();
    let wg = Recorder {
        order: Some(order.clone()),
        ..Recorder::default()
    };
    let fw = FwRecorder {
        order: Some(order.clone()),
        listing: Some("set links_hq0 {\n}".into()),
        ..FwRecorder::default()
    };
    let (result, log) = run_with_firewall(path, &wg, &fw);
    assert_eq!(result.unwrap().len(), 3);
    assert_eq!(*order.borrow(), vec!["nft", "wg"]);
    assert_eq!(
        fw.scripts.borrow().as_slice(),
        [
            "flush set inet hq_net links_hq0\nadd element inet hq_net links_hq0 { 10.77.0.2 . 10.77.0.4, 10.77.0.4 . 10.77.0.2 }\n"
        ]
    );
    assert!(log.contains("1 link(s) on hq0 after this run"), "{log}");
    assert!(log.contains("allowed 10.77.0.2 <-> 10.77.0.4"), "{log}");
}

#[test]
fn a_version_2_file_with_no_links_clears_the_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[]));
    let fw = FwRecorder {
        listing: Some(
            "set links_hq0 {\n elements = { 10.77.0.2 . 10.77.0.3, 10.77.0.3 . 10.77.0.2 }\n}"
                .into(),
        ),
        ..FwRecorder::default()
    };
    let (result, log) = run_with_firewall(path, &Recorder::default(), &fw);
    assert!(result.is_ok());
    assert_eq!(
        fw.scripts.borrow().as_slice(),
        ["flush set inet hq_net links_hq0\n"]
    );
    assert!(log.contains("removed 10.77.0.2 <-> 10.77.0.3"), "{log}");
}

#[test]
fn a_version_1_file_never_touches_the_firewall() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text(2));
    let fw = FwRecorder::default();
    let (result, log) = run_with_firewall(path, &Recorder::default(), &fw);
    assert!(result.is_ok());
    assert!(fw.scripts.borrow().is_empty());
    assert!(!log.contains("link"), "{log}");
}

#[test]
fn a_pair_that_is_already_set_is_one_line_in_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let fw = FwRecorder {
        listing: Some(
            "set links_hq0 {\n elements = { 10.77.0.2 . 10.77.0.3, 10.77.0.3 . 10.77.0.2 }\n}"
                .into(),
        ),
        ..FwRecorder::default()
    };
    let (_, log) = run_with_firewall(path, &Recorder::default(), &fw);
    assert!(
        log.contains("hq-net-apply: no link change, 1 link(s) on hq0\n"),
        "{log}"
    );
}

#[test]
fn when_the_set_cannot_be_read_every_pair_is_listed_as_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let fw = FwRecorder::default();
    let (result, log) = run_with_firewall(path, &Recorder::default(), &fw);
    assert!(result.is_ok());
    assert!(log.contains("set 10.77.0.2 <-> 10.77.0.3"), "{log}");
}

#[test]
fn a_firewall_that_fails_does_not_stop_the_peers_but_fails_the_run_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let wg = Recorder::default();
    let fw = FwRecorder {
        fail_set: true,
        ..FwRecorder::default()
    };
    let (result, log) = run_with_firewall(path, &wg, &fw);
    assert_eq!(
        wg.calls.borrow().len(),
        1,
        "a revocation must not wait for nft"
    );
    let err = result.unwrap_err();
    assert!(matches!(err, ApplyError::Links(_)), "{err}");
    assert!(log.contains("could not set the links on hq0"), "{log}");
    assert!(log.contains("peers were still applied"), "{log}");
}

#[test]
fn a_wg_failure_after_the_pairs_were_set_still_records_the_pairs() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let wg = Recorder {
        fail_apply: true,
        ..Recorder::default()
    };
    let fw = FwRecorder::default();
    let (result, log) = run_with_firewall(path, &wg, &fw);
    assert!(matches!(result.unwrap_err(), ApplyError::Wg(_)));
    assert!(log.contains("wg failed"), "{log}");
    assert!(log.contains("1 link(s) on hq0 after this run"), "{log}");
}

#[test]
fn a_refused_version_2_file_reaches_neither_wg_nor_the_firewall() {
    let dir = tempfile::tempdir().unwrap();
    let bad = [
        valid_text_v2(2, &[(2, 9)]),
        valid_text_v2(2, &[(3, 2)]),
        valid_text_v2(2, &[(2, 3), (2, 3)]),
        valid_text_v2(2, &[(2, 3)]).replace("links 1", "links 2"),
        valid_text_v2(2, &[(2, 3)]).replace("B = 10.77.0.3/32", "PostUp = x"),
    ];
    for (n, text) in bad.iter().enumerate() {
        let path = dir.path().join(format!("peers{n}.conf"));
        write(&path, text.as_bytes(), 0o600);
        let wg = Recorder::default();
        let fw = FwRecorder::default();
        let (result, _) = run_with_firewall(path, &wg, &fw);
        assert!(matches!(result, Err(ApplyError::Rejected(_))), "case {n}");
        assert!(
            wg.calls.borrow().is_empty() && fw.scripts.borrow().is_empty(),
            "case {n}"
        );
        assert_eq!(*wg.shown.borrow(), 0, "case {n}: not even wg showconf");
    }
}

#[test]
fn the_nft_path_is_taken_once_and_must_be_absolute_and_roots() {
    let config = |extra: &[&str]| {
        let mut args: Vec<String> = [
            "--interface",
            "hq0",
            "--subnet",
            "10.77.0.0/24",
            "--hub",
            "10.77.0.1",
            "--owner-uid",
            "1000",
            "--input",
            "/var/lib/hq-net/hq0.conf",
        ]
        .map(String::from)
        .to_vec();
        args.extend(extra.iter().map(|a| a.to_string()));
        parse_args(args)
    };
    assert_eq!(
        config(&["--nft", "/opt/nft"]).unwrap().nft,
        Some(PathBuf::from("/opt/nft"))
    );
    assert!(config(&["--nft", "/a", "--nft", "/b"]).is_err());
    assert!(config(&["--nft"]).is_err());
    assert_eq!(config(&[]).unwrap().nft, None);

    for bad in ["nft", "./nft", "/nonexistent/nft"] {
        let nft = SystemNft::new(Some(Path::new(bad)));
        assert!(nft.resolve().is_err(), "{bad}");
    }
    // Writable by others: refused even though it exists.
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("nft");
    write(&fake, b"#!/bin/sh\n", 0o777);
    assert!(SystemNft::new(Some(&fake)).resolve().is_err());
}

fn script_nft(dir: &Path, body: &str) -> SystemNft {
    let path = dir.join("nft");
    write(&path, format!("#!/bin/sh\n{body}\n").as_bytes(), 0o700);
    SystemNft {
        explicit: None,
        resolved: Some(path),
    }
}

#[test]
fn the_real_nft_spawn_path_feeds_the_script_over_stdin_with_f_dash_and_an_empty_environment() {
    let dir = tempfile::tempdir().unwrap();
    let (script_out, env_out) = (dir.path().join("script"), dir.path().join("env"));
    let nft = script_nft(
        dir.path(),
        &format!(
            "[ \"$1\" = -f ] && [ \"$2\" = - ] && [ $# -eq 2 ] || exit 3\ncat > {}\n/usr/bin/env > {} 2>/dev/null",
            script_out.display(),
            env_out.display()
        ),
    );
    let script = "flush set inet hq_net links_hq0\n";
    nft.set_links("hq0", script).unwrap();
    assert_eq!(std::fs::read_to_string(&script_out).unwrap(), script);
    let env = std::fs::read_to_string(&env_out).unwrap_or_default();
    assert!(!env.contains("HOME=") && !env.contains("CARGO"), "{env}");
}

#[test]
fn listing_asks_for_exactly_the_named_set_and_returns_what_nft_prints() {
    let dir = tempfile::tempdir().unwrap();
    let nft = script_nft(
        dir.path(),
        "[ \"$1 $2 $3 $4 $5\" = 'list set inet hq_net links_hq0' ] || exit 3\nprintf 'set links_hq0 {}'",
    );
    assert_eq!(nft.list_links("hq0").unwrap(), "set links_hq0 {}");
    assert!(
        nft.list_links("hq0 -x").is_err(),
        "an invalid name never reaches nft"
    );
}

#[test]
fn a_failing_nft_is_an_error_that_names_the_risk() {
    let dir = tempfile::tempdir().unwrap();
    let nft = script_nft(dir.path(), "cat >/dev/null; exit 1");
    let err = nft
        .set_links("hq0", "flush set inet hq_net links_hq0\n")
        .unwrap_err();
    assert!(
        err.contains("nothing limits which devices can reach each other"),
        "{err}"
    );
    assert!(nft.list_links("hq0").is_err());
}

#[test]
fn a_hung_nft_is_stopped_at_the_deadline_not_left_to_systemd() {
    let dir = tempfile::tempdir().unwrap();
    let nft = script_nft(dir.path(), "exec sleep 30");
    let started = std::time::Instant::now();
    let err = nft
        .run(&["-f", "-"], Some(b"x"), true, Duration::from_millis(300))
        .unwrap_err();
    assert!(err.contains("did not finish"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_firewall_failure_in_one_round_is_not_washed_away_by_a_later_version_1_round() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let wg = Recorder::default();
    let target = path.clone();
    let mut rewrote = false;
    *wg.during_apply.borrow_mut() = Some(Box::new(move || {
        if !rewrote {
            rewrote = true;
            std::fs::remove_file(&target).unwrap();
            write(&target, valid_text(3).as_bytes(), 0o600);
        }
    }));
    let fw = FwRecorder {
        fail_set: true,
        ..FwRecorder::default()
    };
    let (result, _) = run_with_firewall(path, &wg, &fw);
    assert_eq!(wg.calls.borrow().len(), 2, "both rounds reached wg");
    assert!(matches!(result.unwrap_err(), ApplyError::Links(_)));
}

#[test]
fn a_later_version_2_round_that_sets_the_pairs_clears_an_earlier_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let wg = Recorder::default();
    let target = path.clone();
    let mut rewrote = false;
    *wg.during_apply.borrow_mut() = Some(Box::new(move || {
        if !rewrote {
            rewrote = true;
            std::fs::remove_file(&target).unwrap();
            write(&target, valid_text_v2(3, &[(2, 3)]).as_bytes(), 0o600);
        }
    }));
    let fw = FwRecorder {
        fail_first: std::cell::Cell::new(true),
        ..FwRecorder::default()
    };
    let (result, log) = run_with_firewall(path, &wg, &fw);
    assert_eq!(fw.scripts.borrow().len(), 2);
    assert_eq!(result.unwrap().len(), 3, "{log}");
}

#[test]
fn a_new_key_at_a_linked_address_is_called_out_in_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let show = |seed: u8| {
        format!(
            "[Interface]\nListenPort = 51820\nPrivateKey = {}\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.2/32\n",
            k(SHOWCONF_KEY_SEED),
            k(seed)
        )
    };
    // The interface has another key at .2 than the file does.
    let wg = Recorder {
        show_text: Some(show(50)),
        ..Recorder::default()
    };
    let (_, log) = run_with_firewall(path, &wg, &FwRecorder::default());
    assert!(
        log.contains("note: the device at 10.77.0.2 is in a link and now has a different key"),
        "{log}"
    );
    // The same key at .2: nothing to call out.
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[(2, 3)]));
    let wg = Recorder {
        show_text: Some(show(1)),
        ..Recorder::default()
    };
    let (_, log) = run_with_firewall(path, &wg, &FwRecorder::default());
    assert!(!log.contains("different key"), "{log}");
}

#[test]
fn clearing_a_set_that_cannot_be_read_is_still_journaled() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_in(&dir, &valid_text_v2(2, &[]));
    let (result, log) = run_with_firewall(path, &Recorder::default(), &FwRecorder::default());
    assert!(result.is_ok());
    assert!(log.contains("cleared the list"), "{log}");
}
