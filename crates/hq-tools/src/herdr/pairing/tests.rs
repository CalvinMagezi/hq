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
    assert_eq!(host.kind, HostKind::Native);
    assert_eq!(host.ssh, "dev@100.64.0.9");
    assert_eq!(loaded.default_model, "relay", "other settings are kept");
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
