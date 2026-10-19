//! Tests for the opener and the run loop. They need no root: the real `wg` is replaced by a
//! recorder, and files are made in a temporary directory owned by the test user.

use super::*;
use crate::key;
use std::cell::RefCell;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
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
    /// Runs inside `apply`, to stand in for the input changing while the applier works.
    during_apply: RefCell<Option<Box<dyn FnMut()>>>,
}

const SHOWCONF_KEY_SEED: u8 = 99;

impl Wg for Recorder {
    fn show_config(&self, _interface: &str) -> Result<String, String> {
        *self.shown.borrow_mut() += 1;
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
fn another_owner_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    assert!(matches!(
        read_input(&path, me() + 1),
        Err(InputError::WrongOwner | InputError::ParentUnsafe)
    ));
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
    assert_eq!(run(&config(path), &wg).unwrap().len(), 2);
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
    assert_eq!(run(&config(path), &wg).unwrap().len(), 0);
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
        let err = run(&config(path), &wg).unwrap_err();
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
fn a_failing_wg_is_reported_and_unreadable_input_never_reaches_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    let wg = Recorder {
        fail_apply: true,
        ..Recorder::default()
    };
    assert!(matches!(run(&config(path), &wg), Err(ApplyError::Wg(_))));
    let wg = Recorder::default();
    let missing = dir.path().join("missing.conf");
    assert!(matches!(
        run(&config(missing), &wg),
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
fn interface_names_match_the_kernels_limit_and_a_safe_alphabet() {
    for ok in ["hq0", "a", "wg_hq", "abcdefghijklmno"] {
        assert!(valid_interface(ok), "{ok}");
    }
    for bad in [
        "",
        "Hq0",
        "1hq",
        "hq.0",
        "hq 0",
        "hq/0",
        "hq0\n",
        "abcdefghijklmnop",
        "hq-0",
        "é",
    ] {
        assert!(!valid_interface(bad), "{bad:?}");
    }
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
    assert_eq!(run(&config(path), &wg).unwrap().len(), 3);
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
    assert!(run(&config(path), &wg).is_ok());
    assert_eq!(wg.calls.borrow().len(), MAX_ROUNDS);
}

#[test]
fn a_file_replaced_by_something_hostile_after_the_first_apply_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.conf");
    write(&path, valid_text(1).as_bytes(), 0o600);
    let wg = Recorder::default();
    let target = path.clone();
    *wg.during_apply.borrow_mut() = Some(Box::new(move || {
        std::fs::remove_file(&target).unwrap();
        write(&target, b"[Interface]\nPostUp = id\n", 0o600);
    }));
    assert!(matches!(
        run(&config(path), &wg),
        Err(ApplyError::Rejected(_))
    ));
    assert_eq!(
        wg.calls.borrow().len(),
        1,
        "only the valid first version reached wg"
    );
}

#[test]
fn the_journal_line_has_the_address_and_a_short_key_fingerprint_only() {
    let peer = grammar::Peer {
        public_key: k(7),
        address: std::net::Ipv4Addr::new(10, 77, 0, 9),
        preshared_key: Some(k(8)),
    };
    let line = peer_log_line(&peer);
    assert_eq!(
        line,
        format!("hq-net-apply:   10.77.0.9 {}", &k(7)[..FINGERPRINT_CHARS])
    );
    assert!(!line.contains(&k(8)), "a preshared key is never logged");
}

fn script_wg(dir: &Path, body: &str) -> SystemWg {
    let path = dir.join("wg");
    write(&path, format!("#!/bin/sh\n{body}\n").as_bytes(), 0o700);
    SystemWg { path }
}

#[test]
fn the_real_spawn_path_feeds_the_config_over_a_pipe_to_dev_stdin() {
    // A stand-in program: it must be given `syncconf <interface> /dev/stdin` and the text on stdin.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("received");
    let wg = script_wg(
        dir.path(),
        &format!(
            "[ \"$1\" = syncconf ] && [ \"$2\" = hq0 ] && [ \"$3\" = /dev/stdin ] || exit 3\ncat /dev/stdin > {}",
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
