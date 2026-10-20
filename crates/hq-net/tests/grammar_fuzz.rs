//! Seeded randomized tests of the applier grammar. They assert invariants about what is accepted,
//! not only that nothing panics: whatever the mutated bytes are, an accepted file holds nothing
//! but the three allowed settings, every peer obeys the policy, and rendering and parsing again
//! gives the same peers. The seeds are fixed so a failure reproduces; set `HQ_NET_FUZZ_SEED` to explore.

use hq_net::grammar::{
    self, Link, MAX_LINKS, MAX_PEERS, Parsed, Peer, Policy, parse, parse_file, render, render_file,
    render_labeled,
};
use hq_net::key;
use hq_net::links;
use hq_net::subnet::Subnet;
use hq_net::wgconf;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashSet;
use std::net::Ipv4Addr;

const DEFAULT_SEED: u64 = 0x5eed_0001_4851;

/// The seed is fixed so a failure reproduces; set `HQ_NET_FUZZ_SEED` to explore other runs.
fn seed() -> u64 {
    std::env::var("HQ_NET_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SEED)
}
const ROUNDS: usize = 6_000;
const MAX_MUTATIONS: usize = 4;

/// Lines that are never allowed anywhere in a peers file.
const FORBIDDEN_LINES: [&str; 22] = [
    "[Interface]",
    "[interface]",
    "PostUp = /bin/sh -c id",
    "PreUp = x",
    "PostDown = x",
    "PreDown = x",
    "Table = off",
    "SaveConfig = true",
    "PrivateKey = AAAA",
    "ListenPort = 1",
    "Address = 10.77.0.9/32",
    "DNS = 1.1.1.1",
    "Endpoint = example.com:1",
    "PersistentKeepalive = 1",
    "FwMark = 1",
    "MTU = 1",
    "Include = /etc/passwd",
    "publickey = AAAA",
    "AllowedIPs = 0.0.0.0/0",
    "[Peer] [Interface]",
    "PublicKey",
    "= =",
];

const FRAGMENTS: [&str; 16] = [
    "\n",
    "\r",
    "\0",
    "\t",
    "\u{e9}",
    " # ",
    "/0",
    "/24",
    "0.0.0.0",
    "10.77.0.1",
    "10.77.0.255",
    "[",
    "]",
    "=",
    "\\",
    "%i",
];

fn policies() -> Vec<Policy> {
    let make = |net: &str, hub: Ipv4Addr| Policy::new(Subnet::parse(net).unwrap(), hub).unwrap();
    vec![
        make("10.77.0.0/24", Ipv4Addr::new(10, 77, 0, 1)),
        make("192.168.50.0/25", Ipv4Addr::new(192, 168, 50, 1)),
        make("172.16.0.0/16", Ipv4Addr::new(172, 16, 0, 1)),
    ]
}

fn random_key(rng: &mut StdRng) -> String {
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    key::encode(&bytes)
}

fn random_peers(rng: &mut StdRng, policy: &Policy) -> Vec<Peer> {
    let hosts: Vec<Ipv4Addr> = policy
        .subnet()
        .hosts()
        .filter(|a| *a != policy.hub())
        .take(300)
        .collect();
    let count = rng.gen_range(0..=MAX_PEERS.min(hosts.len()));
    let mut picked = HashSet::new();
    let mut peers = Vec::new();
    while peers.len() < count {
        let address = hosts[rng.gen_range(0..hosts.len())];
        if picked.insert(address) {
            peers.push(Peer {
                public_key: random_key(rng),
                address,
                preshared_key: rng.gen_bool(0.3).then(|| random_key(rng)),
            });
        }
    }
    peers
}

fn mutate(rng: &mut StdRng, bytes: &mut Vec<u8>) {
    if bytes.is_empty() {
        bytes.push(rng.r#gen());
        return;
    }
    let at = rng.gen_range(0..bytes.len());
    match rng.gen_range(0..7) {
        0 => bytes[at] = rng.r#gen(),
        1 => bytes[at] ^= 1 << rng.gen_range(0..8),
        2 => {
            bytes.remove(at);
        }
        3 => bytes.truncate(at),
        4 => {
            let fragment = FRAGMENTS[rng.gen_range(0..FRAGMENTS.len())];
            bytes.splice(at..at, fragment.bytes());
        }
        5 => {
            let end = (at + rng.gen_range(1..20)).min(bytes.len());
            let copy = bytes[at..end].to_vec();
            let to = rng.gen_range(0..=bytes.len());
            bytes.splice(to..to, copy);
        }
        _ => {
            let end = (at + rng.gen_range(1..40)).min(bytes.len());
            bytes.drain(at..end);
        }
    }
}

/// What an accepted input must look like, checked on the input bytes themselves rather than on
/// anything the parser produced.
fn assert_only_allowed_lines(input: &[u8]) {
    assert!(
        input
            .iter()
            .all(|b| *b == b'\n' || (0x20..=0x7e).contains(b)),
        "bytes outside printable ASCII were accepted"
    );
    let text = std::str::from_utf8(input).unwrap();
    assert!(text.ends_with('\n'));
    for line in text.lines().skip(1) {
        let line = line.trim_matches(' ');
        if line.is_empty() || line.starts_with('#') || line == "[Peer]" {
            continue;
        }
        let name = line.split('=').next().unwrap().trim_matches(' ');
        assert!(
            ["PublicKey", "AllowedIPs", "PresharedKey"].contains(&name) && line.contains('='),
            "an accepted file holds a line that is not allowed: {line:?}"
        );
    }
}

const SHOWCONF: &str = "[Interface]\nListenPort = 51820\nPrivateKey = AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\n\n[Peer]\nPublicKey = AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=\nAllowedIPs = 10.77.0.200/32\n";

/// What `wg` would see in the text it is given, read the way wireguard-tools reads a config: each
/// line cut at the first `#`, all whitespace deleted, empty lines skipped, section names and keys
/// compared without regard to case. Returns (section, key, value) for every setting.
fn wg_view(text: &str) -> Vec<(String, String, String)> {
    let mut section = String::new();
    let mut settings = Vec::new();
    for raw in text.lines() {
        let line: String = raw
            .split('#')
            .next()
            .unwrap()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.to_ascii_lowercase();
            settings.push((section.clone(), String::new(), String::new()));
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .expect("wg would reject a line without =");
        settings.push((section.clone(), key.to_ascii_lowercase(), value.to_string()));
    }
    settings
}

/// The text built for `wg` from an accepted file holds exactly one interface section, whose only
/// settings are the three carried from the running interface, and one peer section per accepted
/// peer with exactly the three allowed settings, an address that obeys the policy, and nothing else.
fn assert_wg_sees_only_the_peers(peers: &[Peer], policy: &Policy) {
    let (_, text) = wgconf::build(peers, SHOWCONF).unwrap();
    let view = wg_view(&text);
    let sections: Vec<&str> = view
        .iter()
        .filter(|s| s.1.is_empty())
        .map(|s| s.0.as_str())
        .collect();
    assert_eq!(sections.first(), Some(&"interface"));
    assert_eq!(sections.iter().filter(|s| **s == "interface").count(), 1);
    assert_eq!(
        sections.iter().filter(|s| **s == "peer").count(),
        peers.len()
    );
    assert_eq!(
        sections.len(),
        1 + peers.len(),
        "no other section may exist"
    );
    let mut seen_keys = HashSet::new();
    for (section, key, value) in view.iter().filter(|s| !s.1.is_empty()) {
        match section.as_str() {
            "interface" => assert!(
                ["privatekey", "listenport", "fwmark"].contains(&key.as_str()),
                "{key}"
            ),
            "peer" => assert!(
                ["publickey", "presharedkey", "allowedips"].contains(&key.as_str()),
                "{key}"
            ),
            other => panic!("setting in section {other}"),
        }
        if key == "publickey" {
            assert!(seen_keys.insert(value.clone()));
        }
        if key == "allowedips" {
            let address: Ipv4Addr = value.strip_suffix("/32").expect("a /32").parse().unwrap();
            assert!(policy.subnet().is_usable_host(address) && address != policy.hub());
        }
    }
    let wanted: HashSet<String> = peers.iter().map(|p| p.public_key.clone()).collect();
    assert_eq!(seen_keys, wanted);
}

fn assert_accepted_is_sound(input: &[u8], peers: &[Peer], policy: &Policy) {
    assert_only_allowed_lines(input);
    assert_wg_sees_only_the_peers(peers, policy);
    assert!(peers.len() <= MAX_PEERS);
    let mut addresses = HashSet::new();
    let mut keys = HashSet::new();
    for peer in peers {
        assert!(policy.subnet().is_usable_host(peer.address));
        assert_ne!(peer.address, policy.hub());
        assert!(key::is_usable(&peer.public_key));
        assert!(peer.preshared_key.as_deref().is_none_or(key::is_usable));
        assert!(addresses.insert(peer.address), "duplicate address accepted");
        assert!(
            keys.insert(peer.public_key.clone()),
            "duplicate key accepted"
        );
    }
    assert_eq!(
        parse(render(peers).as_bytes(), policy).as_deref(),
        Ok(peers)
    );
}

#[test]
fn mutated_valid_files_are_refused_or_sound() {
    let mut rng = StdRng::seed_from_u64(seed());
    let policies = policies();
    let (mut accepted, mut refused) = (0usize, 0usize);
    for round in 0..ROUNDS {
        let policy = &policies[round % policies.len()];
        let peers = random_peers(&mut rng, policy);
        let labels: Vec<String> = (0..peers.len()).map(|i| format!("device-{i}")).collect();
        let entries: Vec<(&str, &Peer)> = labels
            .iter()
            .map(String::as_str)
            .zip(peers.iter())
            .collect();
        let mut bytes = render_labeled(&entries).into_bytes();
        assert_eq!(
            parse(&bytes, policy).as_deref(),
            Ok(peers.as_slice()),
            "the unmutated file must parse"
        );
        for _ in 0..rng.gen_range(1..=MAX_MUTATIONS) {
            mutate(&mut rng, &mut bytes);
        }
        match parse(&bytes, policy) {
            Ok(parsed) => {
                accepted += 1;
                assert_accepted_is_sound(&bytes, &parsed, policy);
            }
            Err(_) => refused += 1,
        }
    }
    assert!(
        refused > ROUNDS / 4,
        "the mutations should mostly break files: {refused} of {ROUNDS}"
    );
    assert!(
        accepted > 0,
        "some benign mutations (a comment byte) should still pass: {accepted}"
    );
}

#[test]
fn a_forbidden_line_anywhere_in_a_valid_file_is_always_refused() {
    let mut rng = StdRng::seed_from_u64(seed() ^ 1);
    let policies = policies();
    for round in 0..ROUNDS {
        let policy = &policies[round % policies.len()];
        let peers = random_peers(&mut rng, policy);
        let text = render(&peers);
        let mut lines: Vec<&str> = text.lines().collect();
        let forbidden = FORBIDDEN_LINES[rng.gen_range(0..FORBIDDEN_LINES.len())];
        // Anywhere between the header and the trailer: after the trailer it would be refused for
        // the trailer, not for the rule under test.
        let at = rng.gen_range(1..lines.len());
        lines.insert(at, forbidden);
        let mutated = lines.join("\n") + "\n";
        assert!(
            parse(mutated.as_bytes(), policy).is_err(),
            "accepted {forbidden:?} at line {}",
            at + 1
        );
    }
}

#[test]
fn random_bytes_and_random_printable_lines_never_panic_and_are_sound_if_accepted() {
    let mut rng = StdRng::seed_from_u64(seed() ^ 2);
    let policies = policies();
    for round in 0..ROUNDS {
        let policy = &policies[round % policies.len()];
        let len = rng.gen_range(0..600);
        let mut bytes: Vec<u8> = if round % 2 == 0 {
            (0..len).map(|_| rng.r#gen()).collect()
        } else {
            let mut text = format!("{}\n", grammar::HEADER).into_bytes();
            for _ in 0..rng.gen_range(0..12) {
                let width = rng.gen_range(0..90);
                text.extend((0..width).map(|_| rng.gen_range(0x20u8..0x7f)));
                text.push(b'\n');
            }
            text
        };
        if round % 3 == 0 {
            bytes.extend_from_slice(b"\n[Peer]\nPublicKey = ");
            bytes.extend_from_slice(random_key(&mut rng).as_bytes());
            bytes.extend_from_slice(b"\nAllowedIPs = 10.77.0.2/32\n");
        }
        if let Ok(parsed) = parse(&bytes, policy) {
            assert_accepted_is_sound(&bytes, &parsed, policy);
        }
    }
}

#[test]
fn a_file_cut_short_at_any_byte_is_always_refused() {
    let mut rng = StdRng::seed_from_u64(seed() ^ 3);
    let policy = &policies()[0];
    for _ in 0..60 {
        let peers = random_peers(&mut rng, policy);
        let bytes = render(&peers).into_bytes();
        for cut in 0..bytes.len() {
            assert!(
                parse(&bytes[..cut], policy).is_err(),
                "a file cut at byte {cut} of {} was accepted",
                bytes.len()
            );
        }
    }
}

/// A random set of links among `peers`, each pair once.
fn random_links(rng: &mut StdRng, peers: &[Peer]) -> Vec<Link> {
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    if peers.len() < 2 {
        return links;
    }
    for _ in 0..rng.gen_range(0..=12usize.min(MAX_LINKS)) {
        let x = peers[rng.gen_range(0..peers.len())].address;
        let y = peers[rng.gen_range(0..peers.len())].address;
        if let Some(link) = Link::new(x, y)
            && seen.insert(link)
        {
            links.push(link);
        }
    }
    links
}

/// What an accepted version 2 file must look like, checked on the bytes: the version 1 rules plus
/// the two link settings, and no forwarding language of any other kind.
fn assert_only_allowed_lines_v2(input: &[u8]) {
    assert!(
        input
            .iter()
            .all(|b| *b == b'\n' || (0x20..=0x7e).contains(b)),
        "bytes outside printable ASCII were accepted"
    );
    let text = std::str::from_utf8(input).unwrap();
    assert!(text.ends_with('\n'));
    for line in text.lines().skip(1) {
        let line = line.trim_matches(' ');
        if line.is_empty() || line.starts_with('#') || line == "[Peer]" || line == "[Link]" {
            continue;
        }
        let name = line.split('=').next().unwrap().trim_matches(' ');
        assert!(
            ["PublicKey", "AllowedIPs", "PresharedKey", "A", "B"].contains(&name)
                && line.contains('='),
            "an accepted file holds a line that is not allowed: {line:?}"
        );
    }
}

/// Whatever was accepted, the pairs the hub would be told to allow are exactly the addresses of
/// admitted peers, in order, once each, and the nft script holds nothing else.
fn assert_accepted_v2_is_sound(input: &[u8], parsed: &Parsed, policy: &Policy) {
    assert_only_allowed_lines_v2(input);
    assert_accepted_is_sound_peers(&parsed.peers, policy);
    let Some(parsed_links) = &parsed.links else {
        panic!("a version 2 file must carry a list of links, even an empty one");
    };
    assert!(parsed_links.len() <= MAX_LINKS);
    let addresses: HashSet<Ipv4Addr> = parsed.peers.iter().map(|p| p.address).collect();
    let mut seen = HashSet::new();
    for link in parsed_links {
        assert!(
            link.a < link.b,
            "a link must be in order and between two devices"
        );
        assert!(addresses.contains(&link.a) && addresses.contains(&link.b));
        assert!(seen.insert(*link), "a pair was accepted twice");
    }
    let script = links::script("hq0", parsed_links).unwrap();
    for word in script.split(|c: char| " ,{}\n".contains(c)) {
        if let Ok(address) = word.parse::<Ipv4Addr>() {
            assert!(addresses.contains(&address), "the script names {address}");
        } else {
            assert!(
                [
                    "flush",
                    "set",
                    "inet",
                    "hq_net",
                    "links_hq0",
                    "add",
                    "element",
                    ".",
                    ""
                ]
                .contains(&word),
                "unexpected word in the script: {word:?}"
            );
        }
    }
}

fn assert_accepted_is_sound_peers(peers: &[Peer], policy: &Policy) {
    let mut addresses = HashSet::new();
    let mut keys = HashSet::new();
    for peer in peers {
        assert!(policy.subnet().is_usable_host(peer.address));
        assert_ne!(peer.address, policy.hub());
        assert!(key::is_usable(&peer.public_key));
        assert!(addresses.insert(peer.address));
        assert!(keys.insert(peer.public_key.clone()));
    }
}

#[test]
fn mutated_valid_version_2_files_are_refused_or_sound() {
    let mut rng = StdRng::seed_from_u64(seed() ^ 4);
    let policies = policies();
    let (mut accepted, mut refused) = (0usize, 0usize);
    for round in 0..ROUNDS {
        let policy = &policies[round % policies.len()];
        let peers = random_peers(&mut rng, policy);
        let wanted = random_links(&mut rng, &peers);
        let labels: Vec<String> = (0..peers.len()).map(|i| format!("device-{i}")).collect();
        let entries: Vec<(&str, &Peer)> = labels
            .iter()
            .map(String::as_str)
            .zip(peers.iter())
            .collect();
        let mut bytes = render_file(&entries, Some(&wanted)).into_bytes();
        let parsed = parse_file(&bytes, policy).expect("the unmutated file must parse");
        assert_eq!(parsed.peers, peers);
        assert_eq!(parsed.links.as_deref(), Some(wanted.as_slice()));
        assert!(
            parse(&bytes, policy).is_err(),
            "the version 1 reader must refuse it"
        );
        for _ in 0..rng.gen_range(1..=MAX_MUTATIONS) {
            mutate(&mut rng, &mut bytes);
        }
        match parse_file(&bytes, policy) {
            Ok(parsed) if parsed.links.is_some() => {
                accepted += 1;
                assert_accepted_v2_is_sound(&bytes, &parsed, policy);
            }
            Ok(parsed) => {
                accepted += 1;
                assert_accepted_is_sound(&bytes, &parsed.peers, policy);
            }
            Err(_) => refused += 1,
        }
    }
    assert!(refused > ROUNDS / 4, "{refused} of {ROUNDS}");
    assert!(accepted > 0, "{accepted}");
}

#[test]
fn a_forbidden_line_anywhere_in_a_valid_version_2_file_is_always_refused() {
    let mut rng = StdRng::seed_from_u64(seed() ^ 5);
    let policies = policies();
    let extra: [&str; 5] = [
        "[Link] [Peer]",
        "A = 10.77.0.2/32, 10.77.0.3/32",
        "C = 10.77.0.2/32",
        "Ports = 22",
        "Policy = accept",
    ];
    for round in 0..ROUNDS {
        let policy = &policies[round % policies.len()];
        let peers = random_peers(&mut rng, policy);
        let wanted = random_links(&mut rng, &peers);
        let entries: Vec<(&str, &Peer)> = peers.iter().map(|p| ("", p)).collect();
        let text = render_file(&entries, Some(&wanted));
        let mut lines: Vec<&str> = text.lines().collect();
        let all: Vec<&str> = FORBIDDEN_LINES
            .iter()
            .chain(extra.iter())
            .copied()
            .collect();
        let forbidden = all[rng.gen_range(0..all.len())];
        let at = rng.gen_range(1..lines.len());
        lines.insert(at, forbidden);
        let mutated = lines.join("\n") + "\n";
        assert!(
            parse_file(mutated.as_bytes(), policy).is_err(),
            "accepted {forbidden:?} at line {}",
            at + 1
        );
    }
}

#[test]
fn a_version_2_file_cut_short_at_any_byte_is_always_refused() {
    let mut rng = StdRng::seed_from_u64(seed() ^ 6);
    let policy = &policies()[0];
    for _ in 0..60 {
        let peers = random_peers(&mut rng, policy);
        let wanted = random_links(&mut rng, &peers);
        let entries: Vec<(&str, &Peer)> = peers.iter().map(|p| ("", p)).collect();
        let bytes = render_file(&entries, Some(&wanted)).into_bytes();
        for cut in 0..bytes.len() {
            assert!(
                parse_file(&bytes[..cut], policy).is_err(),
                "a version 2 file cut at byte {cut} of {} was accepted",
                bytes.len()
            );
        }
    }
}
