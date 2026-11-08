use super::*;

const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEXAMPLEKEYBLOB hq-gate";

#[test]
fn the_plist_names_the_binary_and_escapes_it() {
    let plist = launchd_plist(Path::new("/opt/a&b/.local/bin/hq"), Path::new("/opt/a&b"));
    assert!(plist.contains(
        "<string>/opt/a&amp;b/.local/bin/hq</string><string>host</string><string>serve</string>"
    ));
    assert!(plist.contains("<key>KeepAlive</key><true/>"));
    assert!(plist.contains("/opt/a&amp;b/Library/Logs/hq-host.log"));
}

#[test]
fn the_unit_runs_the_host_and_restarts_it() {
    let unit = systemd_unit(
        Path::new("/srv/example/.local/bin/hq"),
        Path::new("/srv/example"),
    );
    assert!(unit.contains("ExecStart=/srv/example/.local/bin/hq host serve"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains(":/srv/example/.local/bin:"));
}

/// An agent can write `~/.local/bin` and `~/bin`; a `git` planted there must not win over
/// the system's in a service that runs outside the sandbox.
#[test]
fn the_services_path_puts_the_users_own_directories_last() {
    let path = path_env(Path::new("/srv/example"));
    let dirs: Vec<&str> = path.split(':').collect();
    let at = |d: &str| dirs.iter().position(|x| *x == d).unwrap();
    for user in [
        "/srv/example/.local/bin",
        "/srv/example/bin",
        "/opt/homebrew/bin",
    ] {
        for system in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
            assert!(at(system) < at(user), "{system} before {user}: {path}");
        }
    }
    assert!(at("/usr/local/bin") < at("/srv/example/.local/bin"));
}

#[test]
fn the_vault_and_config_the_installer_used_go_into_the_service() {
    let env = [
        ("HQ_VAULT_PATH", "/srv/my vault/100%".to_string()),
        ("HQ_CONFIG_PATH", "/srv/a\"b/config.yaml".to_string()),
    ];
    let unit = systemd_unit_with(
        Path::new("/srv/example/.local/bin/hq"),
        Path::new("/srv/example"),
        &env,
    );
    assert!(
        unit.contains("Environment=\"HQ_VAULT_PATH=/srv/my vault/100%%\"\n"),
        "{unit}"
    );
    assert!(
        unit.contains("Environment=\"HQ_CONFIG_PATH=/srv/a\\\"b/config.yaml\"\n"),
        "{unit}"
    );
    let plist = host_agent(Path::new("/h")).plist_with(Path::new("/h/hq"), Path::new("/h"), &env);
    assert!(plist.contains("<key>HQ_VAULT_PATH</key><string>/srv/my vault/100%</string>"));
    assert!(plist.contains("<key>HQ_CONFIG_PATH</key><string>/srv/a\"b/config.yaml</string>"));
    // Nothing set, nothing written.
    let bare = systemd_unit_with(Path::new("/h/hq"), Path::new("/h"), &[]);
    assert!(!bare.contains("HQ_"), "{bare}");
}

#[test]
fn the_authorized_line_pins_the_command_and_the_source() {
    let line = authorized_line(Path::new("/h/.local/bin/hq"), KEY, "100.64.0.1").unwrap();
    assert_eq!(
        line,
        format!("restrict,from=\"100.64.0.1\",command=\"/h/.local/bin/hq host gate\" {KEY}")
    );
}

#[test]
fn keys_and_sources_that_could_smuggle_options_are_refused() {
    let exe = Path::new("/h/hq");
    for bad in [
        "",
        "not-a-key AAAA",
        "ssh-ed25519",
        "command=\"x\" ssh-ed25519 AAAA",
        "ssh-ed25519 AAAA\nssh-ed25519 BBBB",
        "ssh-ed25519 AA\"AA",
    ] {
        assert!(authorized_line(exe, bad, "1.2.3.4").is_err(), "{bad:?}");
    }
    for bad in ["", "1.2.3.4\",command=\"x", "a b", "1.2.3.4\n"] {
        assert!(authorized_line(exe, KEY, bad).is_err(), "{bad:?}");
    }
    assert!(authorized_line(Path::new("/h/\"hq"), KEY, "1.2.3.4").is_err());
}

#[test]
fn appending_is_idempotent_private_and_keeps_existing_lines() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".ssh/authorized_keys");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "ssh-ed25519 AAAAOTHER other").unwrap();
    let line = authorized_line(Path::new("/h/hq"), KEY, "1.2.3.4").unwrap();
    assert!(append_authorized(&file, &line).unwrap());
    assert!(
        !append_authorized(&file, &line).unwrap(),
        "same key twice adds nothing"
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("ssh-ed25519 AAAAOTHER other\n"));
    assert_eq!(text.matches("EXAMPLEKEYBLOB").count(), 1);
    let dir_mode = std::fs::metadata(file.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700);
}

#[test]
fn hostnames_become_valid_host_names() {
    assert_eq!(default_host_name("immy-oac"), "immy-oac");
    assert_eq!(
        default_host_name("Dev-MacBook-Pro.local"),
        "dev-macbook-pro-local"
    );
    assert_eq!(default_host_name("123box"), "host-123box");
    assert_eq!(default_host_name("local"), "machine");
    assert_eq!(default_host_name("native"), "machine");
    assert_eq!(default_host_name("---"), "host-");
    assert!(default_host_name(&"a".repeat(80)).len() <= 32);
}

#[test]
fn a_launch_agent_carries_its_arguments_and_no_keys_or_process_type() {
    let agent = LaunchAgent {
        label: "com.example.daemon",
        args: &["start", "all"],
        stdout: PathBuf::from("/h/Library/Logs/out.log"),
        stderr: PathBuf::from("/h/Library/Logs/err.log"),
    };
    let plist = agent.plist(Path::new("/h/.local/bin/hq"), Path::new("/h"));
    assert!(
        plist.contains(
            "<string>/h/.local/bin/hq</string><string>start</string><string>all</string>"
        )
    );
    assert!(plist.contains("<string>com.example.daemon</string>"));
    assert!(plist.contains("/h/Library/Logs/err.log"));
    assert!(!plist.contains("API_KEY") && !plist.contains("ProcessType"));
}
