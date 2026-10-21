//! Two peers facing each other in one process, each a real WireGuard session and a real stack,
//! joined by lists of datagrams and a virtual clock. B opens a reach connection, which crosses the
//! tunnel and is accepted by A's serve entry: that exercises the handshake, both directions of TCP,
//! the fence, and (with the mock clock) a rekey.

use boringtun::x25519::{PublicKey, StaticSecret};
use hq_tunnel::config::TunnelConfig;
use hq_tunnel::join::{JoinConfig, parse};
use hq_tunnel::pipe::Pipe;
use hq_tunnel::pipe::mem::{self, MemConnector, MemPipe};
use hq_tunnel::secret::Secret32;
use hq_tunnel::session::{Decapsulated, Session};
use hq_tunnel::stack::Stack;
use mock_instant::MockClock;
use rand::rngs::OsRng;
use std::io::ErrorKind;
use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(5);
const PIPE_CAPACITY: usize = 1 << 20;
const A_ADDR: &str = "10.77.0.4";
const B_ADDR: &str = "10.77.0.2";
const SERVE_PORT: u16 = 8443;

struct Node {
    session: Session,
    stack: Stack,
    connector: MemConnector,
    out: Vec<Vec<u8>>,
}

struct World {
    a: Node,
    b: Node,
    base: Instant,
    elapsed: Duration,
    /// While set, every datagram is lost, as if the network between the peers were gone.
    partitioned: bool,
}

fn join(own: &str, hub: &str, hub_key: &[u8; 32]) -> JoinConfig {
    parse(&format!(
        "[Interface]\nAddress = {own}/32\n\n[Peer]\nPublicKey = {}\nEndpoint = vpn.example.com:51820\nAllowedIPs = {hub}/32\n",
        hq_net::key::encode(hub_key)
    ))
    .unwrap()
}

fn config(extra: &str, serve: &str, reach: &str) -> TunnelConfig {
    TunnelConfig::from_toml(&format!("{extra}\n{serve}\n{reach}")).unwrap()
}

fn serve_from(sources: &[&str]) -> String {
    let list: Vec<String> = sources.iter().map(|s| format!("\"{s}\"")).collect();
    format!(
        "[[serve]]\nport = {SERVE_PORT}\nto = \"127.0.0.1:9000\"\nfrom = [{}]\n",
        list.join(", ")
    )
}

fn reach_to_a(port: u16) -> String {
    format!("[[reach]]\nlisten = \"127.0.0.1:47801\"\nto = \"{A_ADDR}:{port}\"\n")
}

impl World {
    fn new(a_cfg: TunnelConfig, b_cfg: TunnelConfig) -> Self {
        let sk_a = StaticSecret::random_from_rng(OsRng);
        let sk_b = StaticSecret::random_from_rng(OsRng);
        let pk_a = PublicKey::from(&sk_a).to_bytes();
        let pk_b = PublicKey::from(&sk_b).to_bytes();
        let base = Instant::now();
        let make =
            |sk: &StaticSecret, peer: [u8; 32], own: &str, other: &str, cfg: &TunnelConfig| Node {
                session: Session::new(&Secret32::new(sk.to_bytes()), peer, None, Some(25)),
                stack: Stack::new(&join(own, other, &peer), cfg, base).unwrap(),
                connector: MemConnector::default(),
                out: vec![],
            };
        Self {
            a: make(&sk_a, pk_b, A_ADDR, B_ADDR, &a_cfg),
            b: make(&sk_b, pk_a, B_ADDR, A_ADDR, &b_cfg),
            base,
            elapsed: Duration::ZERO,
            partitioned: false,
        }
    }

    fn now(&self) -> Instant {
        self.base + self.elapsed
    }

    fn advance(&mut self, by: Duration) {
        self.elapsed += by;
        MockClock::advance(by);
    }

    fn work(node: &mut Node, now: Instant) {
        // What the real loop does on every pass.
        if let Some(d) = node.session.update_timers() {
            node.out.push(d);
        }
        node.stack.poll(now, &mut node.connector);
        for packet in node.stack.take_egress() {
            if let Some(d) = node.session.encapsulate(&packet) {
                node.out.push(d);
            }
        }
    }

    fn deliver(from: &mut Node, to: &mut Node, lose: bool) -> bool {
        let datagrams = std::mem::take(&mut from.out);
        let moved = !datagrams.is_empty();
        if lose {
            return false;
        }
        for d in datagrams {
            let mut out = Decapsulated::default();
            to.session.decapsulate(None, &d, &mut out);
            to.out.extend(out.replies);
            for p in out.packets {
                to.stack.ingress(&p);
            }
        }
        moved
    }

    fn step(&mut self) {
        self.advance(STEP);
        let now = self.now();
        Self::work(&mut self.a, now);
        Self::work(&mut self.b, now);
        for _ in 0..50 {
            let one = Self::deliver(&mut self.a, &mut self.b, self.partitioned);
            let two = Self::deliver(&mut self.b, &mut self.a, self.partitioned);
            if !one && !two {
                break;
            }
            Self::work(&mut self.a, now);
            Self::work(&mut self.b, now);
        }
    }

    fn run_until(&mut self, limit: Duration, mut done: impl FnMut(&mut World) -> bool) -> bool {
        let end = self.elapsed + limit;
        while self.elapsed < end {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn run_for(&mut self, d: Duration) {
        self.run_until(d, |_| false);
    }

    fn open(&mut self) -> MemPipe {
        let (ours, theirs) = mem::pair(PIPE_CAPACITY);
        let now = self.now();
        self.b.stack.open_reach(0, Box::new(ours), now).unwrap();
        theirs
    }
}

fn read_some(pipe: &mut MemPipe) -> Vec<u8> {
    let mut out = vec![];
    let mut buf = [0u8; 4096];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => panic!("{e}"),
        }
    }
    out
}

fn default_world() -> World {
    World::new(
        config("", &serve_from(&[B_ADDR]), ""),
        config("", "", &reach_to_a(SERVE_PORT)),
    )
}

#[test]
fn a_reach_connection_crosses_the_tunnel_to_a_serve_entry_and_back() {
    let mut w = default_world();
    let mut near = w.open();
    near.write(b"hello over the tunnel").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    let (target, far) = &mut w.a.connector.opened[0];
    assert_eq!(target.to_string(), "127.0.0.1:9000");
    let mut far = std::mem::replace(far, mem::pair(1).0);
    w.run_for(Duration::from_millis(200));
    assert_eq!(read_some(&mut far), b"hello over the tunnel");
    far.write(b"and back").unwrap();
    w.run_for(Duration::from_millis(200));
    assert_eq!(read_some(&mut near), b"and back");
    assert_eq!(w.a.stack.stats.accepted, 1);
    assert_eq!(w.b.stack.stats.reach_opened, 1);
}

#[test]
fn closing_either_end_closes_the_other_and_frees_the_connection() {
    let mut w = default_world();
    let mut near = w.open();
    near.write(b"x").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    let mut far = std::mem::replace(&mut w.a.connector.opened[0].1, mem::pair(1).0);
    near.shutdown_write();
    drop(near);
    w.run_for(Duration::from_millis(500));
    assert_eq!(read_some(&mut far), b"x");
    assert_eq!(
        far.read(&mut [0u8; 8]).unwrap(),
        0,
        "the far end sees the end of the stream"
    );
    far.shutdown_write();
    assert!(
        w.run_until(Duration::from_secs(30), |w| w.a.stack.connections() == 0
            && w.b.stack.connections() == 0)
    );
}

#[test]
fn a_source_the_entry_does_not_name_never_reaches_the_target() {
    let mut w = World::new(
        config("", &serve_from(&["10.77.0.9"]), ""),
        config("", "", &reach_to_a(SERVE_PORT)),
    );
    let mut near = w.open();
    near.write(b"let me in").unwrap();
    w.run_for(Duration::from_secs(8));
    assert!(
        w.a.connector.opened.is_empty(),
        "no connection to the target"
    );
    assert!(
        w.a.stack.stats.dropped > 0,
        "the packets were dropped before the stack"
    );
    assert_eq!(w.a.stack.stats.accepted, 0);
}

#[test]
fn a_source_listed_only_for_another_entry_is_dropped_at_the_door() {
    let serve = format!(
        "{}[[serve]]\nport = 9443\nto = \"127.0.0.1:9001\"\nfrom = [\"10.77.0.9\"]\n",
        serve_from(&[B_ADDR])
    );
    let mut w = World::new(config("", &serve, ""), config("", "", &reach_to_a(9443)));
    let mut near = w.open();
    near.write(b"wrong door").unwrap();
    w.run_for(Duration::from_secs(6));
    assert!(
        w.a.stack.stats.dropped > 0,
        "the SYN never reached the stack"
    );
    assert_eq!(w.a.stack.connections(), 0, "and it took no slot");
    assert!(w.a.connector.opened.is_empty());
    assert_eq!(w.a.stack.stats.accepted, 0);
    assert_eq!(
        near.read(&mut [0u8; 8]).map_err(|e| e.kind()),
        Err(ErrorKind::WouldBlock)
    );
}

#[test]
fn a_target_that_refuses_closes_the_connection() {
    let mut w = default_world();
    w.a.connector.refuse = true;
    let mut near = w.open();
    near.write(b"x").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| w.a.stack.stats.denied > 0));
    w.run_for(Duration::from_secs(1));
    assert_eq!(near.read(&mut [0u8; 8]).map_err(|e| e.kind()), Ok(0));
}

#[test]
fn a_transfer_larger_than_every_buffer_arrives_whole_and_in_order() {
    // Small stack buffers and a small pipe force the rings to wrap many times.
    let mut w = World::new(
        config("tcp_buffer_kb = 16", &serve_from(&[B_ADDR]), ""),
        config("tcp_buffer_kb = 16", "", &reach_to_a(SERVE_PORT)),
    );
    let mut near = w.open();
    near.write(b"x").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    let mut far = std::mem::replace(&mut w.a.connector.opened[0].1, mem::pair(1).0);
    let total = 3 * 1024 * 1024usize;
    let pattern = |i: usize| (i.wrapping_mul(31) ^ (i >> 7)) as u8;
    let (mut sent, mut got_far, mut sent_back, mut got_near) = (1usize, vec![], 0usize, vec![]);
    let mut first = true;
    let end = w.elapsed + Duration::from_secs(600);
    while (got_far.len() < total - 1 + 1 || got_near.len() < total) && w.elapsed < end {
        if sent < total {
            let chunk: Vec<u8> = (sent..(sent + 4096).min(total)).map(&pattern).collect();
            if let Ok(n) = near.write(&chunk) {
                sent += n;
            }
        }
        if sent_back < total {
            let chunk: Vec<u8> = (sent_back..(sent_back + 4096).min(total))
                .map(|i| pattern(i + 1))
                .collect();
            if let Ok(n) = far.write(&chunk) {
                sent_back += n;
            }
        }
        w.step();
        let mut part = read_some(&mut far);
        if first && !part.is_empty() {
            first = false;
            assert_eq!(part.remove(0), b'x');
        }
        got_far.extend(part);
        got_near.extend(read_some(&mut near));
    }
    let expected_far: Vec<u8> = (1..total).map(pattern).collect();
    assert_eq!(got_far.len(), expected_far.len());
    assert!(
        got_far == expected_far,
        "bytes into the target are intact and in order"
    );
    let expected_near: Vec<u8> = (0..total).map(|i| pattern(i + 1)).collect();
    assert!(
        got_near == expected_near,
        "bytes back to the caller are intact and in order"
    );
}

#[test]
fn the_session_is_replaced_after_two_minutes_and_traffic_continues() {
    let mut w = default_world();
    let mut near = w.open();
    near.write(b"one").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    let mut far = std::mem::replace(&mut w.a.connector.opened[0].1, mem::pair(1).0);
    w.run_for(Duration::from_millis(200));
    assert_eq!(read_some(&mut far), b"one");
    // Quiet for just over the rekey time, then traffic again.
    w.run_for(Duration::from_secs(125));
    near.write(b"two").unwrap();
    w.run_for(Duration::from_secs(10));
    assert_eq!(read_some(&mut far), b"two");
    assert!(
        w.b.session.handshake_age().unwrap() < Duration::from_secs(60),
        "a new handshake happened"
    );
}

#[test]
fn many_connections_stop_at_the_limit_and_recover_when_they_close() {
    let mut w = World::new(
        config(
            "max_connections = 3\nidle_listeners = 1",
            &serve_from(&[B_ADDR]),
            "",
        ),
        config("max_connections = 8", "", &reach_to_a(SERVE_PORT)),
    );
    let mut ends = vec![];
    for _ in 0..6 {
        let mut near = w.open();
        near.write(b"x").unwrap();
        ends.push(near);
        w.run_for(Duration::from_millis(300));
    }
    assert_eq!(
        w.a.stack.connections(),
        3,
        "the cap is reached and not passed"
    );
    assert_eq!(w.a.connector.opened.len(), 3);
    // Both ends of every forwarded stream close: the callers and the targets.
    drop(ends);
    w.a.connector.opened.clear();
    assert!(w.run_until(Duration::from_secs(60), |w| w.a.stack.connections() == 0));
    // The freed slots serve a new connection.
    let mut again = w.open();
    again.write(b"y").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
}

#[test]
fn a_caller_is_not_hung_up_on_while_its_connection_is_still_opening() {
    use std::time::Instant;
    // One stack alone: nothing can answer, so the connection stays in its first state.
    let base = Instant::now();
    let cfg = config("", "", &reach_to_a(SERVE_PORT));
    let own = join(B_ADDR, A_ADDR, &[1; 32]);
    let mut stack = Stack::new(&own, &cfg, base).unwrap();
    let (ours, mut theirs) = mem::pair(PIPE_CAPACITY);
    stack.open_reach(0, Box::new(ours), base).unwrap();
    let mut connector = MemConnector::default();
    stack.poll(base, &mut connector);
    stack.poll(base + Duration::from_millis(50), &mut connector);
    assert_eq!(
        theirs.read(&mut [0u8; 8]).map_err(|e| e.kind()),
        Err(ErrorKind::WouldBlock),
        "no end of stream before the connection exists"
    );
}

#[test]
fn an_idle_reach_connection_is_closed_after_its_idle_time() {
    let mut w = World::new(
        config("", &serve_from(&[B_ADDR]), ""),
        config("reach_idle_secs = 30", "", &reach_to_a(SERVE_PORT)),
    );
    let mut near = w.open();
    near.write(b"x").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    w.run_for(Duration::from_secs(20));
    assert_eq!(
        w.b.stack.connections(),
        1,
        "still open before the idle time"
    );
    w.run_for(Duration::from_secs(20));
    assert_eq!(w.b.stack.connections(), 0, "closed after it");
    assert_eq!(near.read(&mut [0u8; 8]).map_err(|e| e.kind()), Ok(0));
}

#[test]
fn a_one_shot_client_that_closes_right_behind_its_data_is_still_served() {
    // The data and the close arrive in the same batch as the last step of the handshake, so the
    // connection is already half closed by the first time the stack looks at it.
    let mut w = default_world();
    let mut near = w.open();
    near.write(b"one shot").unwrap();
    near.shutdown_write();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    let mut far = std::mem::replace(&mut w.a.connector.opened[0].1, mem::pair(1).0);
    w.run_for(Duration::from_millis(300));
    assert_eq!(read_some(&mut far), b"one shot");
    assert_eq!(
        far.read(&mut [0u8; 8]).unwrap(),
        0,
        "and the target sees the close"
    );
    assert_eq!(w.a.stack.stats.accepted, 1);
}

#[test]
fn a_reach_entry_stops_at_its_own_cap() {
    let mut w = World::new(
        config("", &serve_from(&[B_ADDR]), ""),
        config(
            "max_connections = 2\nidle_listeners = 1",
            "",
            &reach_to_a(SERVE_PORT),
        ),
    );
    let _first = w.open();
    let _second = w.open();
    let (ours, _theirs) = mem::pair(PIPE_CAPACITY);
    let now = w.now();
    assert!(w.b.stack.open_reach(0, Box::new(ours), now).is_err());
    assert!(!w.b.stack.can_open_reach(0));
}

#[test]
fn the_tunnel_recovers_by_itself_after_a_long_outage() {
    // Longer than WireGuard keeps trying (90 s of unanswered initiations, then it gives up): the
    // peers must start over on their own when the network comes back, with nobody telling them.
    let mut w = default_world();
    let mut near = w.open();
    near.write(b"before").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    w.run_for(Duration::from_millis(200));
    w.partitioned = true;
    w.run_for(Duration::from_secs(400));
    w.partitioned = false;
    w.run_for(Duration::from_secs(60));
    assert!(
        w.a.session.is_established() && w.b.session.is_established(),
        "both ends have a session again"
    );
    let mut again = w.open();
    again.write(b"after the outage").unwrap();
    assert!(w.run_until(Duration::from_secs(30), |w| w.a.connector.opened.len() == 2));
    let mut far = std::mem::replace(&mut w.a.connector.opened[1].1, mem::pair(1).0);
    w.run_for(Duration::from_millis(300));
    assert_eq!(read_some(&mut far), b"after the outage");
}

#[test]
fn a_tunnel_that_only_sends_keepalives_keeps_a_fresh_session() {
    let mut w = default_world();
    let mut near = w.open();
    near.write(b"x").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
    drop(near);
    w.a.connector.opened.clear();
    // A quarter of an hour with nothing but keepalives.
    w.run_for(Duration::from_secs(900));
    for node in [&w.a, &w.b] {
        let age = node.session.handshake_age().expect("a session");
        assert!(age < Duration::from_secs(180), "the session is {age:?} old");
    }
    let mut again = w.open();
    again.write(b"still there").unwrap();
    assert!(w.run_until(Duration::from_secs(10), |w| {
        !w.a.connector.opened.is_empty()
    }));
}
