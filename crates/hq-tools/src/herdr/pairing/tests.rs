use super::*;

fn join(name: &str) -> Join {
    Join { name: name.into(), user: "dev".into(), addr: "100.64.0.9".into(), os: "linux".into() }
}

#[test]
fn a_join_code_round_trips() {
    let code = encode_join(&join("laptop"));
    assert!(code.starts_with("hqjoin1."));
    assert_eq!(decode_join(&format!("  {code}\n")).unwrap(), join("laptop"));
}

#[test]
fn damaged_or_hostile_codes_are_refused() {
    assert!(decode_join("laptop").is_err());
    assert!(decode_join("hqjoin1.!!!").is_err());
    assert!(decode_join(&format!("hqjoin1.{}", "A".repeat(2000))).is_err());
    for bad in [
        Join { name: "local".into(), ..join("x") },
        Join { name: "native".into(), ..join("x") },
        Join { name: "Has Space".into(), ..join("x") },
        Join { name: "1abc".into(), ..join("x") },
        Join { user: "-oProxyCommand=x".into(), ..join("x") },
        Join { user: "a b".into(), ..join("x") },
        Join { addr: "-oProxyCommand=x".into(), ..join("x") },
        Join { addr: "host;rm".into(), ..join("x") },
        Join { addr: "evil@host".into(), ..join("x") },
        Join { addr: "".into(), ..join("x") },
    ] {
        let code = format!("hqjoin1.{}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&bad).unwrap()));
        assert!(decode_join(&code).is_err(), "{bad:?}");
    }
}

#[test]
fn adding_a_host_writes_the_config_and_a_key_and_is_idempotent() {
    if Command::new("ssh-keygen").arg("-?").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.yaml");
    std::fs::write(&config, "default_model: relay\n").unwrap();
    let code = encode_join(&join("laptop"));
    let first = add_host_at(&config, &code, "100.64.0.1", &dir.path().join("ssh")).unwrap();
    assert!(first.config_changed);
    assert_eq!(first.ssh, "dev@100.64.0.9");
    assert!(first.public_key.starts_with("ssh-ed25519 "));
    assert_eq!(
        first.authorize_command,
        format!("hq host authorize --key '{}' --from 100.64.0.1", first.public_key)
    );
    let loaded = HqConfig::load_from_path(&config).unwrap();
    let host = &loaded.herdr.hosts["laptop"];
    assert_eq!(host.ssh, "dev@100.64.0.9");
    assert_eq!(loaded.default_model, "relay", "other settings are kept");
    let written = std::fs::read_to_string(&config).unwrap();
    assert!(!written.contains("launch_bound_secs"), "defaults are not written into the config:\n{written}");
    let mode = std::fs::metadata(&first.identity_file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    let second = add_host_at(&config, &code, "100.64.0.1", &dir.path().join("ssh")).unwrap();
    assert!(!second.config_changed, "the same machine twice changes nothing");
    assert_eq!(second.public_key, first.public_key, "and keeps its key");
}

#[test]
fn a_gateway_address_that_is_not_an_address_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let code = encode_join(&join("laptop"));
    assert!(add_host_at(&dir.path().join("c.yaml"), &code, "x;y", &dir.path().join("ssh")).is_err());
    assert!(!dir.path().join("c.yaml").exists(), "nothing is written for a refused request");
}

const SAMPLE: &str = "# my config\ndefault_model: relay\n\n# Coding agents\nherdr:\n  default_host: laptop\n  hosts:\n    laptop:\n      ssh: \"me@100.64.0.2\"\n      identity_file: \"/k/one\"\n  # profiles\n  harness_profiles:\n    a: {base: claude-code}\n\nother: 1\n";

fn hosts_of(text: &str) -> serde_yaml::Value {
    serde_yaml::from_str::<serde_yaml::Value>(text).unwrap()["herdr"]["hosts"].clone()
}

#[test]
fn a_host_is_added_beside_existing_ones_and_comments_survive() {
    let out = insert_host(SAMPLE, "pc", "dev@100.64.0.9", "/k/two").unwrap().unwrap();
    assert!(out.contains("# my config") && out.contains("# Coding agents") && out.contains("# profiles"));
    let hosts = hosts_of(&out);
    assert_eq!(hosts["laptop"]["ssh"], "me@100.64.0.2");
    assert_eq!(hosts["pc"]["ssh"], "dev@100.64.0.9");
    assert_eq!(hosts["pc"]["identity_file"], "/k/two");
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    assert_eq!(v["other"], 1);
    assert_eq!(v["herdr"]["harness_profiles"]["a"]["base"], "claude-code");
}

#[test]
fn the_same_entry_twice_changes_nothing_and_a_changed_one_is_replaced() {
    let once = insert_host(SAMPLE, "pc", "dev@100.64.0.9", "/k/two").unwrap().unwrap();
    assert!(insert_host(&once, "pc", "dev@100.64.0.9", "/k/two").unwrap().is_none());
    let moved = insert_host(&once, "pc", "dev@100.64.0.77", "/k/three").unwrap().unwrap();
    let hosts = hosts_of(&moved);
    assert_eq!(hosts["pc"]["ssh"], "dev@100.64.0.77");
    assert_eq!(hosts["pc"]["identity_file"], "/k/three");
    assert_eq!(moved.matches("pc:").count(), 1);
    assert_eq!(hosts["laptop"]["ssh"], "me@100.64.0.2");
}

#[test]
fn a_config_without_herdr_or_hosts_gets_them() {
    let none = insert_host("default_model: relay\n", "pc", "dev@h", "/k").unwrap().unwrap();
    assert_eq!(hosts_of(&none)["pc"]["ssh"], "dev@h");
    assert!(none.contains("default_model: relay"));
    let empty = insert_host("", "pc", "dev@h", "/k").unwrap().unwrap();
    assert_eq!(hosts_of(&empty)["pc"]["ssh"], "dev@h");
    let no_hosts = insert_host("herdr:\n  default_host: native\nnext: 1\n", "pc", "dev@h", "/k").unwrap().unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&no_hosts).unwrap();
    assert_eq!(v["herdr"]["hosts"]["pc"]["ssh"], "dev@h");
    assert_eq!(v["herdr"]["default_host"], "native");
    assert_eq!(v["next"], 1);
    let flow = insert_host("herdr:\n  hosts: {}\n", "pc", "dev@h", "/k").unwrap().unwrap();
    assert_eq!(hosts_of(&flow)["pc"]["ssh"], "dev@h");
}

#[test]
fn inline_sections_are_left_for_a_person() {
    assert!(insert_host("herdr: {hosts: {}}\n", "pc", "dev@h", "/k").is_err());
    assert!(insert_host("herdr:\n  hosts: {a: {ssh: x}}\n", "pc", "dev@h", "/k").is_err());
}
