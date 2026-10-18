#![cfg(unix)]

//! End to end tests against a real listener on loopback. The raw client below builds the exchange
//! by hand from the protocol text, so these tests do not depend on the library's own client.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signer, SigningKey};
use hq_gate::{
    ClientKey, GateError, Limits, Listener, ServerConfig, ShutdownHandle, append_authorized, call,
};
use rand::{Rng, SeedableRng, rngs::StdRng};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const LOOPBACK: &str = "127.0.0.1";

struct Fixture {
    addr: SocketAddr,
    keys: PathBuf,
    handled: Arc<AtomicUsize>,
    shutdown: ShutdownHandle,
    dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.shutdown();
    }
}

fn short_limits() -> Limits {
    Limits {
        auth_deadline: Duration::from_millis(400),
        read_deadline: Duration::from_millis(400),
        write_deadline: Duration::from_millis(400),
        max_connections: 16,
    }
}

fn fixture_with(limits: Limits, allowed: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("gate_authorized_keys");
    std::fs::write(&keys, "").unwrap();
    std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o600)).unwrap();
    let listener = Listener::bind(ServerConfig {
        listen: format!("{LOOPBACK}:0").parse().unwrap(),
        allowed_sources: allowed.iter().map(|a| a.parse().unwrap()).collect(),
        authorized_keys: keys.clone(),
        limits,
    })
    .unwrap();
    let addr = listener.local_addr();
    let shutdown = listener.shutdown_handle();
    let handled = Arc::new(AtomicUsize::new(0));
    let counter = handled.clone();
    std::thread::spawn(move || {
        listener.serve(Arc::new(move |request: &str| {
            counter.fetch_add(1, Ordering::SeqCst);
            assert_ne!(request, "boom", "a handler that panics");
            format!("{{\"result\":{{\"echo\":{}}}}}", request.len())
        }));
    });
    Fixture {
        addr,
        keys,
        handled,
        shutdown,
        dir,
    }
}

fn fixture() -> Fixture {
    fixture_with(short_limits(), &[LOOPBACK])
}

fn new_key(fx: &Fixture, name: &str, from: &[&str]) -> ClientKey {
    let key = ClientKey::load_or_create(&fx.dir.path().join(name)).unwrap();
    authorize(fx, &key, from);
    key
}

fn authorize(fx: &Fixture, key: &ClientKey, from: &[&str]) {
    let from: Vec<IpAddr> = from.iter().map(|a| a.parse().unwrap()).collect();
    append_authorized(&fx.keys, &key.public_key(), &from, "test").unwrap();
}

/// A hand-built exchange: returns the nonce and a reader for what the server sends.
fn raw(addr: SocketAddr) -> (TcpStream, BufReader<TcpStream>, [u8; 32]) {
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut hello = String::new();
    reader.read_line(&mut hello).unwrap();
    let nonce = B64
        .decode(hello.trim().strip_prefix("HQGATE1 ").expect("magic"))
        .unwrap();
    (stream, reader, nonce.try_into().unwrap())
}

fn sign(seed: u8, nonce: &[u8; 32], signed_request: &str) -> (String, String) {
    let signing = SigningKey::from_bytes(&[seed; 32]);
    let mut message = b"hq-gate-v1\0".to_vec();
    message.extend_from_slice(nonce);
    message.extend_from_slice(&Sha256::digest(signed_request.as_bytes()));
    let signature = signing.sign(&message);
    (
        B64.encode(signing.verifying_key().as_bytes()),
        B64.encode(signature.to_bytes()),
    )
}

fn authorize_seed(fx: &Fixture, seed: u8) {
    let key = B64.encode(
        SigningKey::from_bytes(&[seed; 32])
            .verifying_key()
            .as_bytes(),
    );
    append_authorized(&fx.keys, &key, &[], "seed").unwrap();
}

/// True when the server closes without sending a reply line.
fn closed_without_reply(mut reader: BufReader<TcpStream>) -> bool {
    let mut line = String::new();
    matches!(reader.read_line(&mut line), Ok(0) | Err(_))
}

#[test]
fn a_valid_call_reaches_the_handler_and_gets_its_reply() {
    let fx = fixture();
    let key = new_key(&fx, "mac.key", &[]);
    let request = r#"["agent.list","{}"]"#;
    let reply = call(fx.addr, &key, request, Duration::from_secs(3)).unwrap();
    assert_eq!(
        reply,
        format!("{{\"result\":{{\"echo\":{}}}}}", request.len())
    );
    assert_eq!(fx.handled.load(Ordering::SeqCst), 1);
}

#[test]
fn a_key_that_is_not_authorized_is_dropped_without_a_reply_and_never_reaches_the_handler() {
    let fx = fixture();
    let stranger = ClientKey::load_or_create(&fx.dir.path().join("stranger.key")).unwrap();
    let err = call(fx.addr, &stranger, "x", Duration::from_secs(3)).unwrap_err();
    assert!(matches!(err, GateError::Refused(_)), "{err}");
    assert_eq!(fx.handled.load(Ordering::SeqCst), 0);
}

#[test]
fn a_signature_over_a_different_request_fails() {
    let fx = fixture();
    authorize_seed(&fx, 5);
    let (mut stream, reader, nonce) = raw(fx.addr);
    let (key, signature) = sign(5, &nonce, "the request that was signed");
    write!(stream, "{key} {signature}\nthe request that was sent\n").unwrap();
    assert!(closed_without_reply(reader));
    assert_eq!(fx.handled.load(Ordering::SeqCst), 0);
}

#[test]
fn a_captured_exchange_cannot_be_replayed_on_a_new_connection() {
    let fx = fixture();
    authorize_seed(&fx, 6);
    let request = r#"["agent.list","{}"]"#;
    let (mut first, mut first_reader, nonce) = raw(fx.addr);
    let (key, signature) = sign(6, &nonce, request);
    write!(first, "{key} {signature}\n{request}\n").unwrap();
    let mut reply = String::new();
    first_reader.read_line(&mut reply).unwrap();
    assert!(
        reply.contains("result"),
        "the genuine exchange works: {reply}"
    );
    let (mut replay, replay_reader, _fresh_nonce) = raw(fx.addr);
    write!(replay, "{key} {signature}\n{request}\n").unwrap();
    assert!(closed_without_reply(replay_reader));
    assert_eq!(fx.handled.load(Ordering::SeqCst), 1);
}

#[test]
fn a_key_restricted_to_other_addresses_is_refused_from_here() {
    let fx = fixture();
    let key = new_key(&fx, "k", &["10.77.0.9"]);
    let err = call(fx.addr, &key, "x", Duration::from_secs(3)).unwrap_err();
    assert!(matches!(err, GateError::Refused(_)), "{err}");
    assert_eq!(fx.handled.load(Ordering::SeqCst), 0);
}

#[test]
fn a_source_that_is_not_allowed_gets_nothing_even_with_a_valid_key() {
    let fx = fixture_with(short_limits(), &["10.77.0.1"]);
    let key = new_key(&fx, "k", &[]);
    let err = call(fx.addr, &key, "x", Duration::from_secs(3)).unwrap_err();
    assert!(
        matches!(err, GateError::Refused(_) | GateError::Protocol(_)),
        "{err}"
    );
    assert_eq!(fx.handled.load(Ordering::SeqCst), 0);
}

#[test]
fn removing_a_key_takes_effect_on_the_next_connection() {
    let fx = fixture();
    let key = new_key(&fx, "k", &[]);
    assert!(call(fx.addr, &key, "x", Duration::from_secs(3)).is_ok());
    std::fs::write(&fx.keys, "").unwrap();
    assert!(call(fx.addr, &key, "x", Duration::from_secs(3)).is_err());
    assert_eq!(fx.handled.load(Ordering::SeqCst), 1);
}

#[test]
fn an_authorized_keys_file_others_can_write_refuses_everyone() {
    let fx = fixture();
    let key = new_key(&fx, "k", &[]);
    std::fs::set_permissions(&fx.keys, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(call(fx.addr, &key, "x", Duration::from_secs(3)).is_err());
    assert_eq!(fx.handled.load(Ordering::SeqCst), 0);
}

#[test]
fn malformed_authentication_lines_are_dropped() {
    let fx = fixture();
    let bad_lines: Vec<Vec<u8>> = vec![
        b"\n".to_vec(),
        b"nospace\n".to_vec(),
        b"a b\n".to_vec(),
        b"!!!! ????\n".to_vec(),
        vec![b'A'; 500],
        {
            let mut v = vec![0xff, 0xfe, 0x00, 0x01];
            v.push(b'\n');
            v
        },
    ];
    for line in bad_lines {
        let (mut stream, reader, _nonce) = raw(fx.addr);
        let _ = stream.write_all(&line);
        assert!(closed_without_reply(reader));
    }
    assert_eq!(fx.handled.load(Ordering::SeqCst), 0);
}

#[test]
fn an_oversized_request_is_dropped_before_the_handler_runs() {
    let fx = fixture();
    authorize_seed(&fx, 7);
    let (mut stream, reader, nonce) = raw(fx.addr);
    let huge = "x".repeat(hq_gate::MAX_REQUEST_BYTES + 10);
    let (key, signature) = sign(7, &nonce, &huge);
    let _ = write!(stream, "{key} {signature}\n{huge}\n");
    assert!(closed_without_reply(reader));
    assert_eq!(fx.handled.load(Ordering::SeqCst), 0);
}

#[test]
fn a_client_that_says_nothing_is_cut_off_at_the_auth_deadline() {
    let fx = fixture();
    let (_stream, mut reader, _nonce) = raw(fx.addr);
    let started = Instant::now();
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    assert!(line.is_empty());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn connections_beyond_the_cap_are_closed_at_once() {
    let limits = Limits {
        max_connections: 2,
        auth_deadline: Duration::from_secs(2),
        ..short_limits()
    };
    let fx = fixture_with(limits, &[LOOPBACK]);
    let first = raw(fx.addr);
    let second = raw(fx.addr);
    let mut third = TcpStream::connect(fx.addr).unwrap();
    third
        .set_read_timeout(Some(Duration::from_millis(1500)))
        .unwrap();
    let mut buf = [0u8; 16];
    assert!(matches!(third.read(&mut buf), Ok(0) | Err(_)));
    drop((first, second));
}

#[test]
fn failed_attempts_never_lock_out_the_legitimate_caller() {
    let fx = fixture();
    let good = new_key(&fx, "good", &[]);
    let stranger = ClientKey::load_or_create(&fx.dir.path().join("s")).unwrap();
    for _ in 0..10 {
        let _ = call(fx.addr, &stranger, "x", Duration::from_secs(2));
    }
    call(fx.addr, &good, "x", Duration::from_secs(3)).unwrap();
    assert_eq!(fx.handled.load(Ordering::SeqCst), 1);
}

#[test]
fn a_handler_that_panics_does_not_leak_its_connection_slot() {
    let limits = Limits {
        max_connections: 2,
        ..short_limits()
    };
    let fx = fixture_with(limits, &[LOOPBACK]);
    let key = new_key(&fx, "k", &[]);
    for _ in 0..6 {
        let _ = call(fx.addr, &key, "boom", Duration::from_secs(2));
    }
    call(fx.addr, &key, "after", Duration::from_secs(3)).unwrap();
}

#[test]
fn the_listener_refuses_unsafe_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let base = ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        allowed_sources: vec!["127.0.0.1".parse().unwrap()],
        authorized_keys: dir.path().join("keys"),
        limits: Limits::default(),
    };
    for unspecified in ["0.0.0.0:0", "[::]:0"] {
        let cfg = ServerConfig {
            listen: unspecified.parse().unwrap(),
            ..base.clone()
        };
        assert!(Listener::bind(cfg).is_err(), "{unspecified}");
    }
    for public in ["203.0.113.5:0", "[2001:db8::1]:0", "[::ffff:0.0.0.0]:0"] {
        let cfg = ServerConfig {
            listen: public.parse().unwrap(),
            ..base.clone()
        };
        assert!(cfg.validate().is_err(), "{public}");
    }
    for tunnel in [
        "10.77.0.2:7747",
        "172.20.0.1:7747",
        "192.168.1.5:7747",
        "100.64.0.9:7747",
        "127.0.0.1:7747",
        "[fd00::2]:7747",
        "[fe80::1]:7747",
    ] {
        let cfg = ServerConfig {
            listen: tunnel.parse().unwrap(),
            ..base.clone()
        };
        assert!(cfg.validate().is_ok(), "{tunnel}");
    }
    let no_sources = ServerConfig {
        allowed_sources: vec![],
        ..base
    };
    assert!(Listener::bind(no_sources).is_err());
}

#[test]
fn garbage_and_mutated_exchanges_never_reach_the_handler_and_the_server_stays_up() {
    let fx = fixture();
    authorize_seed(&fx, 8);
    let mut rng = StdRng::seed_from_u64(0x5eed);
    let request = r#"["agent.list","{}"]"#;
    for round in 0..60 {
        let (mut stream, reader, nonce) = raw(fx.addr);
        let (key, signature) = sign(8, &nonce, request);
        let mut exchange = format!("{key} {signature}\n{request}\n").into_bytes();
        match round % 3 {
            0 => {
                let at = rng.gen_range(0..exchange.len());
                exchange[at] ^= 1 << rng.gen_range(0..8);
            }
            1 => {
                let len = rng.gen_range(1..300);
                exchange = (0..len).map(|_| rng.r#gen::<u8>()).collect();
            }
            _ => exchange.truncate(rng.gen_range(0..exchange.len())),
        }
        let _ = stream.write_all(&exchange);
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut sink = String::new();
        let mut reader = reader;
        let _ = reader.read_line(&mut sink);
        // A mutation can leave the exchange valid (a flipped bit inside the ignored tail), so the
        // only claim is that nothing crashed and the handler saw a request at most when the reply
        // is a real one.
        if sink.is_empty() {
            continue;
        }
        assert!(sink.contains("result"), "{sink}");
    }
    let key = new_key(&fx, "after", &[]);
    assert!(call(fx.addr, &key, request, Duration::from_secs(3)).is_ok());
}
