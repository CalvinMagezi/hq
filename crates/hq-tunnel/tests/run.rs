//! The real loop against a driver: `run` on a thread with a real UDP socket and real loopback TCP,
//! facing a second peer that this test drives by hand.

use boringtun::x25519::{PublicKey, StaticSecret};
use hq_tunnel::config::TunnelConfig;
use hq_tunnel::join::{JoinConfig, parse};
use hq_tunnel::pipe::Pipe;
use hq_tunnel::pipe::mem::{self, MemConnector, MemPipe};
use hq_tunnel::run::{Exit, RunOptions, Summary, run};
use hq_tunnel::secret::Secret32;
use hq_tunnel::session::{Decapsulated, Session};
use hq_tunnel::stack::Stack;
use rand::rngs::OsRng;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::os::fd::FromRawFd;
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const A_ADDR: &str = "10.77.0.4";
const B_ADDR: &str = "10.77.0.2";
const SERVE_PORT: u16 = 8443;
const LIMIT: Duration = Duration::from_secs(20);

fn join(
    own: &str,
    hub: &str,
    hub_key: &[u8; 32],
    endpoint: &str,
    psk: Option<&[u8; 32]>,
) -> JoinConfig {
    let psk_line = psk
        .map(|k| format!("PresharedKey = {}\n", hq_net::key::encode(k)))
        .unwrap_or_default();
    parse(&format!(
        "[Interface]\nAddress = {own}/32\n\n[Peer]\nPublicKey = {}\n{psk_line}Endpoint = {endpoint}\nAllowedIPs = {hub}/32\nPersistentKeepalive = 25\n",
        hq_net::key::encode(hub_key)
    ))
    .unwrap()
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A loopback server that echoes each connection back, on its own threads.
fn echo_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut s = stream;
                let mut buf = [0u8; 4096];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 || s.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Control {
    write: Option<File>,
}

impl Control {
    fn say(&mut self, line: &str) {
        self.write
            .as_mut()
            .unwrap()
            .write_all(format!("{line}\n").as_bytes())
            .unwrap();
    }
    fn close(&mut self) {
        self.write = None;
    }
}

/// The side under test, started on a thread, and the side this test drives.
struct Rig {
    a: Option<JoinHandle<hq_tunnel::Result<Summary>>>,
    control: Control,
    b: B,
    dir: tempfile::TempDir,
    /// Every form in which a key of this rig could show up in a file or a message.
    secrets: Vec<String>,
}

struct B {
    session: Session,
    stack: Stack,
    connector: MemConnector,
    udp: UdpSocket,
    reply_to: SocketAddr,
}

impl B {
    fn step(&mut self) {
        let now = Instant::now();
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = self.udp.recv_from(&mut buf) {
            let mut out = Decapsulated::default();
            self.session.decapsulate(None, &buf[..n], &mut out);
            if !out.packets.is_empty() || !out.replies.is_empty() {
                self.reply_to = from;
            }
            for d in out.replies {
                let _ = self.udp.send_to(&d, self.reply_to);
            }
            for p in out.packets {
                self.stack.ingress(&p);
            }
        }
        if let Some(d) = self.session.update_timers() {
            let _ = self.udp.send_to(&d, self.reply_to);
        }
        self.stack.poll(now, &mut self.connector);
        for p in self.stack.take_egress() {
            if let Some(d) = self.session.encapsulate(&p) {
                let _ = self.udp.send_to(&d, self.reply_to);
            }
        }
    }

    fn drive(&mut self, limit: Duration, mut done: impl FnMut(&mut B) -> bool) -> bool {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            self.step();
            if done(self) {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        false
    }

    fn open(&mut self) -> MemPipe {
        let (ours, theirs) = mem::pair(1 << 20);
        self.stack
            .open_reach(0, Box::new(ours), Instant::now())
            .unwrap();
        theirs
    }
}

fn status(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("status.json")).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
}

fn events(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("events.log")).unwrap_or_default()
}

fn rig(
    a_serve: &str,
    a_reach: &str,
    b_reach_to_a: bool,
    endpoint: &str,
    override_endpoint: bool,
) -> Rig {
    let sk_a = StaticSecret::random_from_rng(OsRng);
    let sk_b = StaticSecret::random_from_rng(OsRng);
    let pk_a = PublicKey::from(&sk_a).to_bytes();
    let pk_b = PublicKey::from(&sk_b).to_bytes();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_nonblocking(true).unwrap();
    let b_addr = udp.local_addr().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = mpsc::channel();
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let a_cfg = TunnelConfig::from_toml(&format!("{a_serve}\n{a_reach}")).unwrap();
    let b_reach = if b_reach_to_a {
        format!(
            "[[reach]]\nlisten = \"127.0.0.1:{}\"\nto = \"{A_ADDR}:{SERVE_PORT}\"\n",
            free_port()
        )
    } else {
        String::new()
    };
    let b_cfg = TunnelConfig::from_toml(&b_reach).unwrap();
    let psk: [u8; 32] = rand::random();
    let secrets = vec![
        hq_net::key::encode(&sk_a.to_bytes()),
        hex(&sk_a.to_bytes()),
        hq_net::key::encode(&sk_b.to_bytes()),
        hex(&sk_b.to_bytes()),
        hq_net::key::encode(&psk),
        hex(&psk),
    ];
    let opts = RunOptions {
        join: join(A_ADDR, B_ADDR, &pk_b, endpoint, Some(&psk)),
        config: a_cfg,
        private_key: Secret32::new(sk_a.to_bytes()),
        status_path: Some(dir.path().join("status.json")),
        events_path: Some(dir.path().join("events.log")),
        control_fd: Some(fds[0]),
        bind: "127.0.0.1:0".parse().unwrap(),
        endpoint_override: override_endpoint.then_some(b_addr),
        on_ready: Some(Box::new(move |addr| tx.send(addr).unwrap())),
    };
    let a = thread::spawn(move || run(opts));
    let a_addr = rx.recv_timeout(LIMIT).unwrap();
    let b = B {
        session: Session::new(
            &Secret32::new(sk_b.to_bytes()),
            pk_a,
            Some(&Secret32::new(psk)),
            Some(25),
        ),
        stack: Stack::new(
            &join(B_ADDR, A_ADDR, &pk_a, "vpn.example.com:51820", None),
            &b_cfg,
            Instant::now(),
        )
        .unwrap(),
        connector: MemConnector::default(),
        udp,
        reply_to: a_addr,
    };
    Rig {
        secrets,
        a: Some(a),
        control: Control {
            write: Some(unsafe { File::from_raw_fd(fds[1]) }),
        },
        b,
        dir,
    }
}

impl Rig {
    fn finish(&mut self) -> Summary {
        self.control.say("quit");
        self.a.take().unwrap().join().unwrap().unwrap()
    }
}

fn serve_to(target: SocketAddr, from: &str) -> String {
    format!("[[serve]]\nport = {SERVE_PORT}\nto = \"{target}\"\nfrom = [\"{from}\"]\n")
}

fn read_some(pipe: &mut MemPipe) -> Vec<u8> {
    let mut out = vec![];
    let mut buf = [0u8; 4096];
    while let Ok(n) = pipe.read(&mut buf) {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

fn echo_through(rig: &mut Rig, message: &[u8]) {
    let mut near = rig.b.open();
    near.write(message).unwrap();
    let mut got = vec![];
    let ok = rig.b.drive(LIMIT, |_| {
        got.extend(read_some(&mut near));
        got.len() >= message.len()
    });
    assert!(ok, "no echo");
    assert_eq!(got, message);
}

#[test]
fn a_connection_crosses_the_real_loop_to_a_real_target_and_back() {
    let target = echo_server();
    let mut r = rig(
        &serve_to(target, B_ADDR),
        "",
        true,
        "vpn.example.com:51820",
        true,
    );
    echo_through(&mut r, b"through the real loop");
    let dir = r.dir.path().to_path_buf();
    r.b.drive(Duration::from_secs(6), |_| status(&dir)["accepted"] == 1);
    let s = status(r.dir.path());
    assert_eq!(s["version"], 1);
    assert_eq!(s["state"], "up");
    assert_eq!(s["accepted"], 1);
    let summary = r.finish();
    assert_eq!(summary.exit, Exit::Quit);
    assert_eq!(status(r.dir.path())["state"], "stopped");
    assert!(events(r.dir.path()).contains("session up"));
}

#[test]
fn a_reach_listener_forwards_a_real_connection_to_the_other_peer() {
    // Here the roles swap: the peer under test listens on loopback and B serves.
    let sk_a = StaticSecret::random_from_rng(OsRng);
    let sk_b = StaticSecret::random_from_rng(OsRng);
    let pk_a = PublicKey::from(&sk_a).to_bytes();
    let pk_b = PublicKey::from(&sk_b).to_bytes();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_nonblocking(true).unwrap();
    let b_addr = udp.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let port = free_port();
    let a_cfg = TunnelConfig::from_toml(&format!(
        "[[reach]]\nlisten = \"127.0.0.1:{port}\"\nto = \"{B_ADDR}:{SERVE_PORT}\"\n"
    ))
    .unwrap();
    let b_cfg = TunnelConfig::from_toml(&format!(
        "[[serve]]\nport = {SERVE_PORT}\nto = \"127.0.0.1:9\"\nfrom = [\"{A_ADDR}\"]\n"
    ))
    .unwrap();
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let opts = RunOptions {
        join: join(A_ADDR, B_ADDR, &pk_b, "vpn.example.com:51820", None),
        config: a_cfg,
        private_key: Secret32::new(sk_a.to_bytes()),
        status_path: None,
        events_path: None,
        control_fd: Some(fds[0]),
        bind: "127.0.0.1:0".parse().unwrap(),
        endpoint_override: Some(b_addr),
        on_ready: Some(Box::new(move |addr| tx.send(addr).unwrap())),
    };
    let a = thread::spawn(move || run(opts));
    let a_addr = rx.recv_timeout(LIMIT).unwrap();
    let mut b = B {
        session: Session::new(&Secret32::new(sk_b.to_bytes()), pk_a, None, Some(25)),
        stack: Stack::new(
            &join(B_ADDR, A_ADDR, &pk_a, "vpn.example.com:51820", None),
            &b_cfg,
            Instant::now(),
        )
        .unwrap(),
        connector: MemConnector::default(),
        udp,
        reply_to: a_addr,
    };
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.set_nonblocking(true).unwrap();
    client.write_all(b"from a local program").unwrap();
    assert!(b.drive(LIMIT, |b| !b.connector.opened.is_empty()));
    let mut far = std::mem::replace(&mut b.connector.opened[0].1, mem::pair(1).0);
    let mut got = vec![];
    assert!(b.drive(LIMIT, |_| {
        got.extend(read_some(&mut far));
        got.len() >= 20
    }));
    assert_eq!(got, b"from a local program");
    far.write(b"reply").unwrap();
    let mut back = vec![];
    let ok = b.drive(LIMIT, |_| {
        let mut buf = [0u8; 64];
        match client.read(&mut buf) {
            Ok(n) if n > 0 => back.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            _ => {}
        }
        back.len() >= 5
    });
    eprintln!(
        "B conns {} stats {:?} far write ok",
        b.stack.connections(),
        b.stack.stats
    );
    assert!(ok);
    assert_eq!(back, b"reply");
    let mut control = Control {
        write: Some(unsafe { File::from_raw_fd(fds[1]) }),
    };
    control.say("quit");
    assert_eq!(a.join().unwrap().unwrap().exit, Exit::Quit);
}

#[test]
fn the_app_closing_the_control_channel_stops_the_peer() {
    let mut r = rig("", "", false, "vpn.example.com:51820", true);
    r.b.drive(Duration::from_millis(300), |_| false);
    r.control.close();
    let summary = r.a.take().unwrap().join().unwrap().unwrap();
    assert_eq!(summary.exit, Exit::ControlClosed);
}

#[test]
fn a_network_change_moves_the_session_to_a_new_socket_and_traffic_continues() {
    let target = echo_server();
    let mut r = rig(
        &serve_to(target, B_ADDR),
        "",
        true,
        "vpn.example.com:51820",
        true,
    );
    echo_through(&mut r, b"before");
    let before = r.b.reply_to;
    r.control.say("network-changed");
    assert!(
        r.b.drive(LIMIT, |b| b.reply_to != before),
        "the peer now sends from a new port"
    );
    echo_through(&mut r, b"after the change");
    let summary = r.finish();
    assert_eq!(summary.exit, Exit::Quit);
    assert!(events(r.dir.path()).contains("network changed"));
}

#[test]
fn a_target_that_closes_at_once_does_not_make_the_loop_spin() {
    let closer = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = closer.local_addr().unwrap();
    thread::spawn(move || {
        for stream in closer.incoming().flatten() {
            drop(stream);
        }
    });
    let mut r = rig(
        &serve_to(target, B_ADDR),
        "",
        true,
        "vpn.example.com:51820",
        true,
    );
    let mut near = r.b.open();
    near.write(b"anyone there").unwrap();
    r.b.drive(Duration::from_millis(1500), |_| false);
    drop(near);
    r.b.drive(Duration::from_secs(1), |_| false);
    let summary = r.finish();
    assert!(
        summary.wakeups < 250,
        "{} wake-ups in about 2.5 s",
        summary.wakeups
    );
}

#[test]
fn an_endpoint_that_does_not_resolve_is_reported_and_retried_not_fatal() {
    let mut r = rig("", "", false, "does-not-exist.invalid:51820", false);
    thread::sleep(Duration::from_millis(1500));
    let s = status(r.dir.path());
    assert_eq!(s["state"], "connecting");
    assert!(
        s["error"]
            .as_str()
            .unwrap_or_default()
            .contains("did not resolve"),
        "{s}"
    );
    assert_eq!(r.finish().exit, Exit::Quit);
}

#[test]
fn the_status_and_event_files_never_hold_a_key() {
    let target = echo_server();
    let mut r = rig(
        &serve_to(target, B_ADDR),
        "",
        true,
        "vpn.example.com:51820",
        true,
    );
    echo_through(&mut r, b"x");
    // Garbage and a network change push the refusal and event paths through the files too.
    for _ in 0..20 {
        let _ = r.b.udp.send_to(&[7u8; 100], r.b.reply_to);
    }
    r.control.say("network-changed");
    r.b.drive(Duration::from_secs(6), |_| false);
    let secrets = r.secrets.clone();
    let dir = r.dir.path().to_path_buf();
    r.finish();
    for name in ["status.json", "events.log"] {
        let text = std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(!text.is_empty(), "{name} was written");
        for secret in &secrets {
            assert!(!text.contains(secret.as_str()), "{name} holds a key");
        }
    }
}

#[test]
fn datagrams_from_anyone_but_the_endpoint_never_reach_the_session() {
    let target = echo_server();
    let mut r = rig(
        &serve_to(target, B_ADDR),
        "",
        true,
        "vpn.example.com:51820",
        true,
    );
    echo_through(&mut r, b"warm up");
    // A stranger on the network who knows the phone's UDP port sends what looks like handshakes.
    let stranger = UdpSocket::bind("127.0.0.1:0").unwrap();
    for n in 0..300u32 {
        let mut datagram = vec![0u8; 148];
        datagram[0] = 1;
        datagram[4..8].copy_from_slice(&n.to_be_bytes());
        stranger.send_to(&datagram, r.b.reply_to).unwrap();
    }
    r.b.drive(Duration::from_millis(500), |_| false);
    echo_through(&mut r, b"still fine");
    let summary = r.finish();
    // The partner may be refused once on its own (a repeated timestamp); 300 strangers would add
    // a hundred or more.
    assert!(
        summary.refused < 10,
        "the session saw {} refusals",
        summary.refused
    );
}

#[test]
fn a_flood_that_carries_the_endpoints_address_is_not_read_flat_out() {
    // How long the flood lasts, and the fewest datagrams that make it a flood on any machine.
    const FLOOD_FOR: Duration = Duration::from_millis(1_500);
    const AT_LEAST: u64 = 20_000;
    let target = echo_server();
    let mut r = rig(
        &serve_to(target, B_ADDR),
        "",
        true,
        "vpn.example.com:51820",
        true,
    );
    echo_through(&mut r, b"warm up");
    // Sent from the hub's own address and port, so the connected socket lets all of it in. Read
    // flat out that is hundreds of thousands of refusals a second; the loop must stop reading.
    let junk = r.b.udp.try_clone().unwrap();
    let to = r.b.reply_to;
    let sender = thread::spawn(move || {
        let end = Instant::now() + FLOOD_FOR;
        let mut sent = 0u64;
        let mut datagram = [9u8; 100];
        while Instant::now() < end {
            datagram[4..12].copy_from_slice(&sent.to_be_bytes());
            if junk.send_to(&datagram, to).is_ok() {
                sent += 1;
            }
        }
        sent
    });
    r.b.drive(FLOOD_FOR + Duration::from_millis(300), |_| false);
    let sent = sender.join().unwrap();
    assert!(sent > AT_LEAST, "only {sent} datagrams: not a flood");
    echo_through(&mut r, b"alive after the flood");
    let summary = r.finish();
    assert!(summary.read_pauses >= 1, "reading never stopped");
    assert!(
        summary.wakeups < 400,
        "{} wakeups, {} pauses, {} refused for {sent} datagrams: the flood was read flat out",
        summary.wakeups,
        summary.read_pauses,
        summary.refused
    );
    assert!(
        summary.refused < sent / 2,
        "{} refused of {sent}: most of the flood should have been left in the kernel",
        summary.refused
    );
}

#[test]
fn a_flood_from_the_endpoint_costs_the_log_one_summary_not_a_line_each() {
    let target = echo_server();
    let mut r = rig(
        &serve_to(target, B_ADDR),
        "",
        true,
        "vpn.example.com:51820",
        true,
    );
    echo_through(&mut r, b"warm up");
    for n in 0..3000u32 {
        let mut datagram = vec![0u8; 100];
        datagram[0] = 9;
        datagram[4..8].copy_from_slice(&n.to_be_bytes());
        let _ = r.b.udp.send_to(&datagram, r.b.reply_to);
    }
    r.b.drive(Duration::from_millis(500), |_| false);
    echo_through(&mut r, b"alive after the flood");
    let dir = r.dir.path().to_path_buf();
    let summary = r.finish();
    assert_eq!(summary.exit, Exit::Quit, "the loop stayed responsive");
    assert!(summary.refused > 0, "the session did see them");
    let log = events(&dir);
    assert!(log.lines().count() < 15, "{} lines", log.lines().count());
    assert!(log.len() < 2_000, "{} bytes", log.len());
}
