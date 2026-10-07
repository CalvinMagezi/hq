use super::*;
use std::net::TcpListener;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn public(_: &str, _: u16) -> Vec<IpAddr> {
    vec![ip("93.184.216.34")]
}

#[test]
fn private_ranges_are_recognised() {
    for s in [
        "127.0.0.1",
        "10.1.2.3",
        "192.168.0.9",
        "172.16.5.5",
        "169.254.169.254",
        "100.64.0.1",
        "100.127.255.255",
        "198.18.0.1",
        "198.19.255.1",
        "192.0.0.8",
        "64:ff9b::7f00:1",
        "2002:7f00:1::1",
        "::7f00:1",
        "fec0::1",
        "0.0.0.0",
        "224.0.0.1",
        "::1",
        "::",
        "fe80::1",
        "fc00::1",
        "fd12::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
    ] {
        assert!(is_private(ip(s)), "{s} should be private");
    }
    for s in [
        "93.184.216.34",
        "1.1.1.1",
        "100.63.255.255",
        "100.128.0.1",
        "2606:4700::1111",
    ] {
        assert!(!is_private(ip(s)), "{s} should be public");
    }
}

#[test]
fn rule_matching_is_exact_or_subdomain_only() {
    let exact = Rule::new("api.anthropic.com");
    assert!(exact.matches("api.anthropic.com", 443));
    assert!(!exact.matches("api.anthropic.com", 80));
    assert!(!exact.matches("evil.api.anthropic.com", 443));
    assert!(!exact.matches("api.anthropic.com.evil.test", 443));
    let wild = Rule::new("*.example.com");
    assert!(wild.matches("a.example.com", 443));
    assert!(wild.matches("a.b.example.com", 443));
    assert!(!wild.matches("example.com", 443));
    assert!(!wild.matches("badexample.com", 443));
    assert!(Rule::new("x.test").port(8444).matches("x.test", 8444));
    assert!(!Rule::new("x.test").port(8444).matches("x.test", 443));
}

#[test]
fn decide_allows_listed_public_hosts_only() {
    let rules = [Rule::new("api.anthropic.com")];
    assert!(matches!(
        decide(&rules, "api.anthropic.com", 443, public),
        Verdict::Allow(_)
    ));
    assert!(matches!(
        decide(&rules, "API.Anthropic.COM.", 443, public),
        Verdict::Allow(_)
    ));
    assert!(matches!(
        decide(&rules, "example.org", 443, public),
        Verdict::Deny(_)
    ));
    assert!(matches!(
        decide(&rules, "api.anthropic.com", 22, public),
        Verdict::Deny(_)
    ));
}

#[test]
fn decide_refuses_private_resolution_and_dns_tricks() {
    let rules = [Rule::new("api.anthropic.com")];
    let loopback = |_: &str, _: u16| vec![ip("127.0.0.1")];
    assert!(matches!(
        decide(&rules, "api.anthropic.com", 443, loopback),
        Verdict::Deny(_)
    ));
    let mixed = |_: &str, _: u16| vec![ip("93.184.216.34"), ip("169.254.169.254")];
    assert!(matches!(
        decide(&rules, "api.anthropic.com", 443, mixed),
        Verdict::Deny(_)
    ));
    let none = |_: &str, _: u16| vec![];
    assert!(matches!(
        decide(&rules, "api.anthropic.com", 443, none),
        Verdict::Deny(_)
    ));
}

#[test]
fn private_rule_allows_private_resolution() {
    let rules = [Rule::new("hq.tailnet.test").port(8444).private()];
    let tailnet = |_: &str, _: u16| vec![ip("100.101.102.103")];
    assert!(matches!(
        decide(&rules, "hq.tailnet.test", 8444, tailnet),
        Verdict::Allow(_)
    ));
}

#[test]
fn ip_literals_need_an_exact_rule() {
    let rules = [Rule::new("*.example.com")];
    assert!(matches!(
        decide(&rules, "1.1.1.1", 443, public),
        Verdict::Deny(_)
    ));
    assert!(matches!(
        decide(&rules, "127.0.0.1", 443, public),
        Verdict::Deny(_)
    ));
    assert!(matches!(
        decide(&rules, "::1", 443, public),
        Verdict::Deny(_)
    ));
    let exact = [Rule::new("1.1.1.1")];
    let same = |_: &str, _: u16| vec![ip("1.1.1.1")];
    assert!(matches!(
        decide(&exact, "1.1.1.1", 443, same),
        Verdict::Allow(_)
    ));
    let private_literal = [Rule::new("127.0.0.1")];
    let lo = |_: &str, _: u16| vec![ip("127.0.0.1")];
    assert!(matches!(
        decide(&private_literal, "127.0.0.1", 443, lo),
        Verdict::Deny(_)
    ));
}

#[test]
fn authority_parsing_rejects_smuggling() {
    assert_eq!(
        split_authority("a.test:443", None),
        Some(("a.test".into(), 443))
    );
    assert_eq!(
        split_authority("a.test", Some(80)),
        Some(("a.test".into(), 80))
    );
    assert_eq!(
        split_authority("[::1]:8080", None),
        Some(("::1".into(), 8080))
    );
    assert_eq!(split_authority("a.test", None), None);
    for bad in [
        "u@a.test:443",
        "a.test:443/x",
        "a.test:99999",
        "a.test:x",
        "a b:1",
        "a.test%2e:1",
        "",
        "é.test:1",
        "a.test?x:1",
    ] {
        assert_eq!(split_authority(bad, Some(80)), None, "{bad}");
    }
}

/// An upstream that answers every connection with `reply` after reading the head.
fn upstream(reply: &'static [u8]) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2048];
        let _ = s.read(&mut buf);
        let _ = s.write_all(reply);
    });
    (port, handle)
}

fn ask(proxy: u16, request: &str) -> String {
    let mut c = TcpStream::connect((Ipv4Addr::LOCALHOST, proxy)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    c.write_all(request.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn connect_to_allowed_host_tunnels_and_logs() {
    let (up, handle) = upstream(b"HELLO");
    let egress = Egress::new();
    let port = egress
        .open("a", vec![Rule::new("localhost").port(up).private()])
        .unwrap();
    let out = ask(
        port,
        &format!("CONNECT localhost:{up} HTTP/1.1\r\nHost: localhost:{up}\r\n\r\nping"),
    );
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(out.ends_with("HELLO"), "{out}");
    handle.join().unwrap();
    let log = egress.decisions("a");
    assert_eq!(log.len(), 1);
    assert!(log[0].allowed && log[0].host == "localhost");
}

#[test]
fn denied_hosts_get_403_and_are_logged() {
    let egress = Egress::new();
    let port = egress
        .open("a", vec![Rule::new("api.anthropic.com")])
        .unwrap();
    for target in [
        "evil.test:443",
        "127.0.0.1:22",
        "169.254.169.254:80",
        "localhost:443",
    ] {
        let out = ask(port, &format!("CONNECT {target} HTTP/1.1\r\n\r\n"));
        assert!(out.starts_with("HTTP/1.1 403"), "{target}: {out}");
    }
    let log = egress.decisions("a");
    assert_eq!(log.len(), 4);
    assert!(log.iter().all(|d| !d.allowed && !d.reason.is_empty()));
}

#[test]
fn malformed_and_unsupported_requests_are_refused() {
    let egress = Egress::new();
    let port = egress
        .open("a", vec![Rule::new("localhost").private()])
        .unwrap();
    assert!(ask(port, "GARBAGE\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(ask(port, "GET /relative HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(ask(port, "TRACE http://localhost/ HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 405"));
    assert!(ask(port, "CONNECT user@localhost:443 HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(
        egress.decisions("a").is_empty(),
        "nothing malformed reaches the decision stage"
    );
}

#[test]
fn oversized_head_is_refused() {
    let egress = Egress::new();
    let port = egress.open("a", vec![]).unwrap();
    let big = format!(
        "CONNECT a.test:443 HTTP/1.1\r\nX: {}\r\n\r\n",
        "a".repeat(MAX_HEAD_BYTES + 100)
    );
    assert!(ask(port, &big).starts_with("HTTP/1.1 400"));
}

#[test]
fn plain_http_is_forwarded_once_with_a_close() {
    let (up, handle) = upstream(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    let egress = Egress::new();
    let port = egress
        .open("a", vec![Rule::new("localhost").port(up).private()])
        .unwrap();
    let out = ask(
        port,
        &format!(
            "GET http://localhost:{up}/x HTTP/1.1\r\nProxy-Authorization: secret\r\nConnection: keep-alive\r\n\r\n"
        ),
    );
    assert!(out.ends_with("ok"), "{out}");
    handle.join().unwrap();
}

#[test]
fn plain_http_does_not_forward_proxy_credentials() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let up = listener.local_addr().unwrap().port();
    let seen = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2048];
        let n = s.read(&mut buf).unwrap();
        let _ = s.write_all(b"HTTP/1.1 204 No Content\r\n\r\n");
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    let egress = Egress::new();
    let port = egress
        .open("a", vec![Rule::new("localhost").port(up).private()])
        .unwrap();
    ask(
        port,
        &format!(
            "GET http://localhost:{up}/x HTTP/1.1\r\nProxy-Authorization: secret\r\nX-Keep: 1\r\n\r\n"
        ),
    );
    let head = seen.join().unwrap().to_ascii_lowercase();
    assert!(!head.contains("proxy-authorization"), "{head}");
    assert!(
        head.contains("x-keep: 1") && head.contains("connection: close"),
        "{head}"
    );
}

#[test]
fn each_agent_has_its_own_rules_and_close_stops_the_listener() {
    let egress = Egress::new();
    let a = egress.open("a", vec![Rule::new("one.test")]).unwrap();
    let b = egress.open("b", vec![Rule::new("two.test")]).unwrap();
    assert_ne!(a, b);
    assert!(ask(a, "CONNECT two.test:443 HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 403"));
    assert_eq!(egress.decisions("a").len(), 1);
    assert!(egress.decisions("b").is_empty());
    egress.close("a");
    std::thread::sleep(ACCEPT_POLL * 4);
    assert!(TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, a)),
        Duration::from_millis(500)
    )
    .is_err());
    assert_eq!(egress.decisions("a").len(), 1, "the log outlives the listener");
    egress.forget("a");
    assert!(egress.decisions("a").is_empty());
}

#[test]
fn reopening_replaces_the_earlier_listener() {
    let egress = Egress::new();
    let first = egress.open("a", vec![]).unwrap();
    let second = egress.open("a", vec![]).unwrap();
    std::thread::sleep(ACCEPT_POLL * 4);
    assert!(TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, first)),
        Duration::from_millis(500)
    )
    .is_err());
    assert!(TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, second)),
        Duration::from_millis(500)
    )
    .is_ok());
}

#[test]
fn decision_log_is_bounded() {
    let egress = Egress::new();
    let port = egress.open("a", vec![]).unwrap();
    for i in 0..LOG_CAPACITY + 20 {
        ask(port, &format!("CONNECT h{i}.test:443 HTTP/1.1\r\n\r\n"));
    }
    let log = egress.decisions("a");
    assert_eq!(log.len(), LOG_CAPACITY);
    assert_eq!(
        log.last().unwrap().host,
        format!("h{}.test", LOG_CAPACITY + 19)
    );
}

#[test]
fn connection_budget_is_enforced() {
    let egress = Egress::new();
    let port = egress.open("a", vec![]).unwrap();
    // Idle connections that never send a head hold their slots until the head timeout.
    let held: Vec<TcpStream> = (0..MAX_CONNECTIONS)
        .map(|_| TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap())
        .collect();
    std::thread::sleep(Duration::from_secs(1));
    let out = ask(port, "CONNECT a.test:443 HTTP/1.1\r\n\r\n");
    assert!(out.starts_with("HTTP/1.1 503"), "{out}");
    drop(held);
}
