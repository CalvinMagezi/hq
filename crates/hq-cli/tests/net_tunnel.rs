//! `hq net tunnel`, end to end through the real binary: the key arrives on stdin and never
//! appears anywhere it should not, a new hub needs its key named and takes the maps away, and the
//! peer starts, speaks and stops on command. Built only with the `tunnel` feature:
//! `cargo test -p hq-cli --features tunnel --test net_tunnel`.
#![cfg(all(unix, feature = "tunnel"))]

use hq_net::key::{decode, encode, public_key_of};
use std::io::{Read, Write};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const HUB_KEY_BYTE: u8 = 3;
const WAIT: Duration = Duration::from_secs(20);
/// A WireGuard handshake initiation: message type 1, 148 bytes.
const INITIATION_LEN: usize = 148;

struct Home {
    root: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn tunnel_dir(&self) -> PathBuf {
        self.root.path().join("tunnel")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hq"));
        cmd.env_clear()
            .env("HOME", self.root.path())
            .arg("net")
            .arg("tunnel")
            .arg("--tunnel-dir")
            .arg(self.tunnel_dir())
            .args(args);
        cmd
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn import(&self, text: &str, confirm: Option<&str>) -> Output {
        match confirm {
            Some(key) => self.run(&["import", "--confirm-hub", key], text),
            None => self.run(&["import"], text),
        }
    }
}

fn join_text(hub: u8, endpoint: &str) -> String {
    format!(
        "[Interface]\nAddress = 10.77.0.4/32\n\n[Peer]\nPublicKey = {}\nEndpoint = {endpoint}\nAllowedIPs = 10.77.0.1/32\nPersistentKeepalive = 25\n",
        encode(&[hub; 32])
    )
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A private and a public key from the real `keygen --stdout`.
fn keygen_pair(home: &Home) -> (String, String) {
    let out = home.run(&["keygen", "--stdout"], "");
    assert!(out.status.success(), "{}", text(&out));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let mut lines = stdout.lines();
    let private = lines.next().unwrap().to_string();
    let public = lines.next().unwrap().to_string();
    assert!(lines.next().is_none());
    (private, public)
}

fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(start.elapsed() < WAIT, "the peer did not stop");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Kills the child if the test fails halfway, so no peer is left running.
struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_all(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn keygen_prints_a_key_pair_to_a_pipe_and_writes_a_private_file_once() {
    let home = Home::new();
    let (private, public) = keygen_pair(&home);
    assert!(decode(&private).is_some());
    assert_eq!(public_key_of(&private).unwrap(), public);

    let first = home.run(&["keygen"], "");
    assert!(first.status.success(), "{}", text(&first));
    let key_file = home.tunnel_dir().join("key");
    let written = read_all(&key_file);
    let shown = text(&first);
    assert!(
        shown.contains(&public_key_of(written.trim()).unwrap()),
        "the public key is printed"
    );
    assert!(!shown.contains(written.trim()), "the private one is not");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&key_file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    let second = home.run(&["keygen"], "");
    assert!(!second.status.success());
    assert_eq!(
        read_all(&key_file),
        written,
        "an existing key is never replaced"
    );
}

#[test]
fn a_new_hub_is_confirmed_by_its_key_and_takes_the_serve_map_with_it() {
    let home = Home::new();
    let hub_key = encode(&[HUB_KEY_BYTE; 32]);
    let join = join_text(HUB_KEY_BYTE, "vpn.example.com:51820");

    let unconfirmed = home.import(&join, None);
    // Its own status, so an app can tell "ask the owner" from "refused" without reading words.
    assert_eq!(unconfirmed.status.code(), Some(2));
    let shown = text(&unconfirmed);
    assert!(shown.contains(&hub_key), "the key to compare is shown");
    assert!(
        !shown.contains(&format!("--confirm-hub {hub_key}")),
        "no command that confirms itself is offered: {shown}"
    );
    assert!(!home.tunnel_dir().join("join.conf").exists());

    let wrong = home.import(&join, Some(&encode(&[4; 32])));
    assert_eq!(
        wrong.status.code(),
        Some(1),
        "a wrong key is an error, not a question"
    );
    assert!(!home.tunnel_dir().join("join.conf").exists());

    let ok = home.import(&join, Some(&hub_key));
    assert!(ok.status.success(), "{}", text(&ok));

    let maps = home.tunnel_dir().join("maps.toml");
    std::fs::write(
        &maps,
        "[[serve]]\nport = 8443\nto = \"127.0.0.1:9000\"\nfrom = [\"10.77.0.1\"]\n",
    )
    .unwrap();
    let check = home.run(&["check"], "");
    assert!(check.status.success(), "{}", text(&check));
    assert!(text(&check).contains("1 serve"), "{}", text(&check));

    // The same hub again: no confirmation, the map stays.
    assert!(home.import(&join, None).status.success());
    assert!(maps.exists());

    // Another hub: refused without its key, and the map is still there.
    let other = join_text(5, "vpn.example.com:51820");
    assert!(!home.import(&other, None).status.success());
    assert!(maps.exists());
    let moved = home.import(&other, Some(&encode(&[5; 32])));
    assert!(moved.status.success(), "{}", text(&moved));
    assert!(!maps.exists(), "the map written for the old hub is gone");

    // A map that opens a door to a non-loopback host is refused by `check`.
    std::fs::write(
        &maps,
        "[[serve]]\nport = 8443\nto = \"192.0.2.1:9000\"\nfrom = [\"10.77.0.1\"]\n",
    )
    .unwrap();
    assert!(!home.run(&["check"], "").status.success());
}

#[test]
fn a_hostile_join_text_is_refused_and_leaves_the_old_one_in_force() {
    let home = Home::new();
    let hub_key = encode(&[HUB_KEY_BYTE; 32]);
    let join = join_text(HUB_KEY_BYTE, "vpn.example.com:51820");
    assert!(home.import(&join, Some(&hub_key)).status.success());
    let before = read_all(&home.tunnel_dir().join("join.conf"));
    let private = encode(&[1; 32]);
    for bad in [
        format!("{join}PrivateKey = {private}\n"),
        format!("{join}PostUp = rm -rf /\n"),
        "[Interface]\n".to_string(),
    ] {
        let out = home.import(&bad, Some(&hub_key));
        assert_eq!(out.status.code(), Some(1));
        assert!(!text(&out).contains(&private), "the value is never echoed");
        assert_eq!(read_all(&home.tunnel_dir().join("join.conf")), before);
    }
}

#[test]
fn the_peer_reads_its_key_from_stdin_speaks_to_the_hub_and_stops_on_quit() {
    let home = Home::new();
    let hub = UdpSocket::bind("127.0.0.1:0").unwrap();
    hub.set_read_timeout(Some(WAIT)).unwrap();
    let endpoint = format!("127.0.0.1:{}", hub.local_addr().unwrap().port());
    let hub_key = encode(&[HUB_KEY_BYTE; 32]);
    let imported = home.import(&join_text(HUB_KEY_BYTE, &endpoint), Some(&hub_key));
    assert!(imported.status.success(), "{}", text(&imported));
    let (private, _public) = keygen_pair(&home);

    let mut child = Guard(
        home.command(&["run"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut stdin = child.0.stdin.take().unwrap();
    // The key, then a control line in the same write: the key reader must leave the rest alone.
    stdin
        .write_all(format!("{private}\nnetwork-changed\n").as_bytes())
        .unwrap();

    let mut datagram = [0u8; 2048];
    let (n, _) = hub
        .recv_from(&mut datagram)
        .expect("a handshake from the peer");
    assert_eq!(n, INITIATION_LEN);
    assert_eq!(datagram[0], 1, "message type 1, an initiation");
    // The hub stays silent, so the peer tries again after WireGuard's retransmit time. boringtun
    // logs that, and a peer whose output nobody reads would block on it: nothing may be printed.
    let (n, _) = hub.recv_from(&mut datagram).expect("a second try");
    assert_eq!((n, datagram[0]), (INITIATION_LEN, 1));

    stdin.write_all(b"quit\n").unwrap();
    let status = wait_for_exit(&mut child.0);
    assert!(status.success());

    let mut stdout = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status_file = read_all(&home.tunnel_dir().join("status.json"));
    let events = read_all(&home.tunnel_dir().join("events.log"));
    assert!(status_file.contains("\"stopped\""), "{status_file}");
    assert!(
        events.contains("network changed"),
        "the control line after the key was read"
    );

    let key_bytes = decode(&private).unwrap();
    for (what, haystack) in [
        ("stdout", &stdout),
        ("stderr", &stderr),
        ("status.json", &status_file),
        ("events.log", &events),
    ] {
        assert!(!haystack.contains(&private), "{what} holds the key");
        assert!(
            !haystack.contains(&hex(&key_bytes)),
            "{what} holds the key in hex"
        );
    }
    let files = std::fs::read_dir(home.tunnel_dir()).unwrap();
    for entry in files.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != "key" {
            let content = read_all(&entry.path());
            assert!(!content.contains(&private), "{name} holds the key");
            assert!(
                !content.contains(&hex(&key_bytes)),
                "{name} holds the key in hex"
            );
        }
    }
    assert!(stdout.is_empty(), "the peer printed: {stdout}");
    assert!(stderr.is_empty(), "the peer printed: {stderr}");
}

#[test]
fn the_end_of_stdin_stops_the_peer_because_the_app_is_gone() {
    let home = Home::new();
    let hub = UdpSocket::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("127.0.0.1:{}", hub.local_addr().unwrap().port());
    let hub_key = encode(&[HUB_KEY_BYTE; 32]);
    assert!(
        home.import(&join_text(HUB_KEY_BYTE, &endpoint), Some(&hub_key))
            .status
            .success()
    );
    let (private, _) = keygen_pair(&home);
    let mut child = Guard(
        home.command(&["run"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut stdin = child.0.stdin.take().unwrap();
    stdin.write_all(format!("{private}\n").as_bytes()).unwrap();
    drop(stdin);
    assert!(wait_for_exit(&mut child.0).success());
    let mut stderr = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(
        stderr.contains("--no-control"),
        "it says why it stopped: {stderr}"
    );
    assert!(!stderr.contains(&private));
}

#[test]
fn the_output_of_peer_add_can_be_imported_as_it_is() {
    let home = Home::new();
    let hub_key = encode(&[HUB_KEY_BYTE; 32]);
    let config = join_text(HUB_KEY_BYTE, "vpn.example.com:51820");
    let printed = format!(
        "Adding phone \"pixel\" at 10.77.0.4. This changes peers.toml.\n\nConfig for the device:\n{config}\nScan this with the app:\n{}\n\nTreat it like a password.\n",
        "\u{2588}\u{2580}\u{2584} ".repeat(400)
    );
    assert!(printed.len() > 4096, "longer than a join text may be");
    let out = home.import(&printed, Some(&hub_key));
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(read_all(&home.tunnel_dir().join("join.conf")), config);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn a_private_key_is_never_printed_to_a_terminal() {
    use std::os::fd::{FromRawFd, OwnedFd, RawFd};
    let (mut master, mut slave) = (0 as RawFd, 0 as RawFd);
    // SAFETY: both out-pointers are valid; the name, termios and winsize arguments may be null.
    let ok = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(ok, 0);
    // SAFETY: both descriptors were just made and nothing else owns them.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let home = Home::new();
    let out = home
        .command(&["keygen", "--stdout"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(slave))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("terminal"));
    assert!(!home.tunnel_dir().join("key").exists());
    drop(master);
}

#[test]
fn a_tunnel_directory_that_is_open_to_others_is_refused_by_every_command() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let hub_key = encode(&[HUB_KEY_BYTE; 32]);
    let join = join_text(HUB_KEY_BYTE, "vpn.example.com:51820");
    assert!(home.import(&join, Some(&hub_key)).status.success());
    let (private, _) = keygen_pair(&home);
    std::fs::set_permissions(home.tunnel_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    for args in [&["check"][..], &["status"], &["run"], &["import"]] {
        let out = home.run(args, &format!("{private}\n{join}"));
        assert!(!out.status.success(), "{args:?}");
        assert!(text(&out).contains("chmod 700"), "{args:?}: {}", text(&out));
    }
}

#[test]
fn run_refuses_what_is_not_a_key_and_says_nothing_of_it() {
    let home = Home::new();
    let hub_key = encode(&[HUB_KEY_BYTE; 32]);
    assert!(
        home.import(
            &join_text(HUB_KEY_BYTE, "vpn.example.com:51820"),
            Some(&hub_key)
        )
        .status
        .success()
    );
    for input in [
        "",
        "short\n",
        "wrong key text but long enough to fail the decode ok\n",
    ] {
        let out = home.run(&["run"], input);
        assert!(!out.status.success());
        assert!(!text(&out).contains(input.trim()) || input.trim().is_empty());
    }
    let no_control_without_file = home.run(&["run", "--no-control"], "");
    assert!(text(&no_control_without_file).contains("--key-file"));
}

#[test]
fn status_reports_a_stopped_peer_and_a_missing_one() {
    let home = Home::new();
    assert!(!home.run(&["status"], "").status.success(), "no status yet");
    std::fs::create_dir_all(home.tunnel_dir()).unwrap();
    std::fs::set_permissions(
        home.tunnel_dir(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    std::fs::write(
        home.tunnel_dir().join("status.json"),
        r#"{"version":1,"state":"up","address":"10.77.0.4","endpoint":"x:1","updated_unix":1,"last_handshake_age_s":3,"connections":0,"accepted":0,"denied":0,"error":null}"#,
    )
    .unwrap();
    let out = home.run(&["status"], "");
    assert!(out.status.success());
    assert!(text(&out).contains("not running"), "{}", text(&out));
    let json = home.run(&["status", "--json"], "");
    assert!(text(&json).contains("\"state\": \"up\""));
}
