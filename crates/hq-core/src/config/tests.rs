mod load_figment_layering_tests {
    use crate::config::HqConfig;
    use figment::{
        Figment,
        providers::{Format, Serialized, Yaml},
    };

    /// Mirrors `HqConfig::load()`'s exact merge chain (base struct defaults →
    /// config file → env), but with `Yaml::string` instead of `Yaml::file` so
    /// the test needs no filesystem or env vars. Only a partial `relay:`
    /// block is given, the same shape as a real deployment that sets
    /// `telegram_enabled`/`telegram_token` and leaves the tuning fields
    /// untouched — this is the exact case that used to silently resolve to
    /// zero-value fields instead of their `#[serde(default = "fn")]` values,
    /// because the base layer baked in `RelayConfig`'s derived (naive)
    /// `Default` ahead of the fix in relay.rs.
    #[test]
    fn partial_relay_block_keeps_custom_defaults_through_figment() {
        let yaml = "relay:\n  telegram_enabled: true\n  telegram_token: \"abc\"\n";

        let config: HqConfig = Figment::from(Serialized::defaults(HqConfig::default()))
            .merge(Yaml::string(yaml))
            .extract()
            .unwrap();

        assert!(config.relay.telegram_enabled);
        assert_eq!(config.relay.telegram_token.as_deref(), Some("abc"));
        assert_eq!(config.relay.turn_ack_timeout_secs, 270);
        assert_eq!(config.relay.background_turn_max_days, 5);
        assert!(config.relay.summarize_session_exits);
        assert_eq!(config.relay.session_exit_summary_timeout_secs, 90);
    }

    /// Automatic sub-agent follow-up is opt-in: an empty config leaves it off.
    #[test]
    fn subagent_followup_is_off_by_default_with_a_daily_cap() {
        let config: HqConfig = Figment::from(Serialized::defaults(HqConfig::default()))
            .merge(Yaml::string("{}"))
            .extract()
            .unwrap();
        assert!(!config.collaboration.supervision_followup);
        assert_eq!(config.collaboration.followups_per_chat_per_day, 48);
    }

    /// The README's local-model example must stay a config HQ actually loads.
    #[test]
    fn readme_local_backend_example_loads() {
        let readme = include_str!("../../../../README.md");
        let section = &readme[readme.find("### Running models locally").unwrap()..];
        let start = section.find("```yaml").unwrap() + "```yaml".len();
        let yaml = &section[start..start + section[start..].find("```").unwrap()];

        let config: HqConfig = Figment::from(Serialized::defaults(HqConfig::default()))
            .merge(Yaml::string(yaml))
            .extract()
            .unwrap();

        assert_eq!(config.backends.primary, "local");
        let entry = &config.backends.backends[0];
        assert_eq!(entry.endpoint.as_deref(), Some("http://localhost:11434/v1"));
        assert_eq!(entry.credential_env.as_deref(), Some("OLLAMA_API_KEY"));
    }

    /// Confirms an `instance.features` flag reaches the config through the
    /// same Figment merge chain `HqConfig::load()` uses.
    #[test]
    fn partial_instance_block_overrides_only_the_named_feature() {
        let yaml = "instance:\n  features:\n    local_ollama: false\n";

        let config: HqConfig = Figment::from(Serialized::defaults(HqConfig::default()))
            .merge(Yaml::string(yaml))
            .extract()
            .unwrap();

        assert!(!config.instance.features.local_ollama);
    }
}

mod load_from_path_tests {
    use crate::config::{HqConfig, InstanceType};

    fn write_config(yaml: &str) -> tempfile::TempPath {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, yaml.as_bytes()).unwrap();
        file.into_temp_path()
    }

    /// A fresh install with zero config file, zero env vars, zero cloud
    /// keys, zero Ollama: `local_only` is off by default (no config lock-in
    /// to a local-only assumption), and `default_model` is the generic
    /// `"relay"` router alias rather than a literal Ollama tag.
    #[test]
    fn missing_config_file_yields_generic_zero_config_defaults() {
        let missing = std::env::temp_dir().join("hq-config-does-not-exist.yaml");
        let config = crate::HqConfig::load_from_path(&missing).unwrap();
        assert!(!config.local_only);
        assert_eq!(config.default_model, "relay");
        assert_eq!(config.instance.instance_type, InstanceType::Local);
        assert!(config.instance.features.local_ollama);
        assert!(config.companies.is_empty());
        assert!(config.default_company.is_empty());
        assert!(config.http_referer.is_none());
    }

    #[test]
    fn http_referer_comes_from_config_yaml() {
        let path = write_config("http_referer: https://hq.example.com\n");
        let config = crate::HqConfig::load_from_path(&path).unwrap();
        assert_eq!(config.http_referer.as_deref(), Some("https://hq.example.com"));
    }

    /// `instance_type: cloud` with no `features:` block at all should pick
    /// up every `InstanceFeatures::cloud()` value (every hardware flag
    /// off) — not the `Local` defaults a naive Figment merge would produce
    /// because the base `Serialized::defaults` layer always populates
    /// `features.*` before the YAML layer is even merged in.
    #[test]
    fn cloud_instance_type_with_no_features_block_gets_cloud_feature_defaults() {
        let path = write_config("instance:\n  instance_type: cloud\n");
        let config = crate::HqConfig::load_from_path(&path).unwrap();
        assert_eq!(config.instance.instance_type, InstanceType::Cloud);
        assert!(!config.instance.features.local_ollama);
    }

    /// Explicit `instance_type: local` (or its absence) is unaffected by
    /// the cloud-detection probe.
    #[test]
    fn explicit_local_instance_type_keeps_local_defaults() {
        let path = write_config("instance:\n  instance_type: local\n");
        let config = crate::HqConfig::load_from_path(&path).unwrap();
        assert_eq!(config.instance.instance_type, InstanceType::Local);
        assert!(config.instance.features.local_ollama);
    }

    /// `HQ_RELAY__DISCORD_TOKEN`-style nested
    /// env vars (double underscore denotes nesting) reach nested config sections,
    /// not just top-level fields.
    #[test]
    fn nested_env_var_overrides_nested_config_section() {
        // SAFETY: test-only process-env mutation; no other test in this
        // module touches these env keys or runs config loading
        // concurrently against them.
        unsafe {
            std::env::set_var("HQ_RELAY__DISCORD_TOKEN", "from-env-token");
        }
        let missing = std::env::temp_dir().join("hq-config-does-not-exist-2.yaml");
        let config = crate::HqConfig::load_from_path(&missing).unwrap();
        unsafe {
            std::env::remove_var("HQ_RELAY__DISCORD_TOKEN");
        }
        assert_eq!(
            config.relay.discord_token.as_deref(),
            Some("from-env-token")
        );
    }

    /// `save_patch` must rebuild from the file's own layer, not from an
    /// env-merged `load()` result — otherwise an operator who follows the
    /// "keep secrets in env, not committed-shaped YAML" recommendation
    /// (see the nested-env-var test above) would have that same secret
    /// written straight back into the plaintext file the moment they ran
    /// `hq onboard`/`hq env` to add an unrelated key.
    #[test]
    fn save_patch_never_writes_an_env_supplied_secret_into_the_file() {
        let path = write_config("vault_path: \"/tmp/whatever\"\n");
        unsafe {
            std::env::set_var("HQ_DEEPSEEK_API_KEY", "env-only-secret");
        }
        let result = HqConfig::save_patch_to_path(&path, |c| {
            c.openrouter_api_key = Some("typed-openrouter-key".to_string());
        });
        unsafe {
            std::env::remove_var("HQ_DEEPSEEK_API_KEY");
        }
        let updated = result.unwrap();

        // The patch closure explicitly set this — it's expected in memory
        // and in the file.
        assert_eq!(
            updated.openrouter_api_key.as_deref(),
            Some("typed-openrouter-key")
        );

        // The env-only secret must NOT have been absorbed into the saved
        // struct, and must NOT appear in the file on disk.
        assert_eq!(updated.deepseek_api_key, None);
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("env-only-secret"));
    }
}

mod serde_yaml_none_field_probe {
    //! Answers a specific question load-bearing for
    //! `HqConfig::warn_unrecognized_top_level_keys`: does
    //! `serde_yaml::to_value` on a struct with `Option::None` fields OMIT
    //! those keys (which would make the known-key comparison set
    //! incomplete and cause false-positive "unrecognized key" warnings on
    //! a hand-written config.yaml that legitimately sets one of them), or
    //! does it emit them as `null` (keeping the comparison sound)?
    use crate::config::HqConfig;

    #[test]
    fn none_option_fields_still_appear_as_keys_not_omitted() {
        let value = serde_yaml::to_value(HqConfig::default()).unwrap();
        let map = value.as_mapping().unwrap();
        // openai_api_key is None in HqConfig::default() — if serde_yaml
        // omitted None fields, this key would be absent.
        assert!(
            map.contains_key("openai_api_key"),
            "serde_yaml::to_value omits None Option fields — \
             warn_unrecognized_top_level_keys's known-key set would be \
             incomplete and warn on legitimately-set optional fields"
        );
    }
}

mod bash_config_tests {
    use crate::config::{BashSandboxMode, HqConfig};
    use figment::{
        Figment,
        providers::{Format, Serialized, Yaml},
    };

    fn load(yaml: &str) -> HqConfig {
        Figment::from(Serialized::defaults(HqConfig::default()))
            .merge(Yaml::string(yaml))
            .extract()
            .unwrap()
    }

    #[test]
    fn defaults_are_required_with_no_passthrough() {
        let config = load("");
        assert!(config.governance.bash.env_passthrough.is_empty());
        assert_eq!(config.governance.bash.sandbox, BashSandboxMode::Required);
        assert!(config.governance.bash.network);
    }

    // YAML 1.1 reads a bare `off` as a boolean, which would reject the
    // documented spelling, so every mode goes through the real loader.
    #[test]
    fn every_sandbox_mode_parses_bare() {
        for (text, mode) in [
            ("off", BashSandboxMode::Off),
            ("best_effort", BashSandboxMode::BestEffort),
            ("required", BashSandboxMode::Required),
        ] {
            let config = load(&format!("governance:\n  bash:\n    sandbox: {text}\n"));
            assert_eq!(config.governance.bash.sandbox, mode, "mode `{text}`");
        }
    }

    #[test]
    fn passthrough_list_and_partial_governance_block() {
        let config = load(
            "governance:\n  skills_write_approval: true\n  bash:\n    env_passthrough: [GH_TOKEN, GITHUB_TOKEN]\n",
        );
        assert!(config.governance.skills_write_approval);
        assert!(config.governance.background_review);
        assert_eq!(
            config.governance.bash.env_passthrough,
            vec!["GH_TOKEN".to_string(), "GITHUB_TOKEN".to_string()]
        );
    }
}

#[test]
fn prefer_server_path_uses_server_only_when_user_path_missing() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user.yaml");
    let server = dir.path().join("server.yaml");

    assert_eq!(super::prefer_server_path(user.clone(), &server), user);

    std::fs::write(&server, "x: 1").unwrap();
    assert_eq!(super::prefer_server_path(user.clone(), &server), server);

    std::fs::write(&user, "x: 2").unwrap();
    assert_eq!(super::prefer_server_path(user.clone(), &server), user);
}

#[test]
fn read_path_falls_back_to_the_server_but_an_explicit_path_never_does() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user.yaml");
    let server = dir.path().join("server.yaml");
    std::fs::write(&server, "x: 1").unwrap();

    assert_eq!(super::read_path(None, user.clone(), &server), server);
    let explicit = dir.path().join("explicit.yaml");
    assert_eq!(
        super::read_path(Some(explicit.clone()), user, &server),
        explicit,
        "HQ_CONFIG_PATH is authoritative even when the file is missing"
    );
}

#[test]
fn a_patch_writes_the_path_it_was_given_not_the_server_file() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user.yaml");
    let server = dir.path().join("server.yaml");
    std::fs::write(&server, "default_model: \"server\"\n").unwrap();

    super::HqConfig::save_patch_to_path(&user, |c| c.default_model = "mine".into()).unwrap();

    assert!(std::fs::read_to_string(&user).unwrap().contains("mine"));
    assert_eq!(
        std::fs::read_to_string(&server).unwrap(),
        "default_model: \"server\"\n"
    );
}

#[cfg(unix)]
#[test]
fn prefer_server_path_ignores_a_server_file_this_user_cannot_read() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user.yaml");
    let server = dir.path().join("server.yaml");
    std::fs::write(&server, "x: 1").unwrap();
    std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root can still open a 0o000 file; the assertion only holds for an unprivileged user.
    if std::fs::File::open(&server).is_err() {
        assert_eq!(super::prefer_server_path(user.clone(), &server), user);
    }
    std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(unix)]
#[test]
fn saved_config_and_its_new_directory_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hq-home/config.yaml");

    super::HqConfig::save_patch_to_path(&path, |c| c.default_model = "m".into()).unwrap();
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    super::HqConfig::save_patch_to_path(&path, |c| c.default_model = "n".into()).unwrap();
    assert_eq!(
        mode(&path),
        0o600,
        "a loose existing config is tightened on save"
    );
}

#[test]
fn debug_output_of_config_types_never_contains_a_secret() {
    use super::HqConfig;
    const SECRETS: [&str; 5] = [
        "or-key-AAAA1111",
        "web-token-BBBB2222",
        "discord-CCCC3333",
        "mcp-key-DDDD4444",
        "tg-EEEE5555",
    ];
    let yaml = format!(
        "vault_path: /tmp/v\nopenrouter_api_key: {}\nweb_auth_token: {}\nrelay:\n  discord_token: {}\n  telegram_token: {}\nremote_mcp:\n  - name: d\n    url: https://example.com/mcp\n    api_key: {}\n",
        SECRETS[0], SECRETS[1], SECRETS[2], SECRETS[4], SECRETS[3]
    );
    let path = std::env::temp_dir().join(format!("hq-debug-redact-{}.yaml", std::process::id()));
    std::fs::write(&path, yaml).unwrap();
    let cfg = HqConfig::file_layer_figment(&path)
        .extract::<HqConfig>()
        .unwrap();
    std::fs::remove_file(&path).ok();

    let (logs, _guard) = crate::test_util::capture_logs();
    tracing::warn!(config = ?cfg, "loaded");
    tracing::warn!(relay = ?cfg.relay, "relay");
    tracing::warn!(mcp = ?cfg.remote_mcp[0], "mcp");
    let printed = format!(
        "{cfg:?}{:?}{:?}{}",
        cfg.relay,
        cfg.remote_mcp,
        logs.contents()
    );

    for secret in SECRETS {
        assert!(!printed.contains(secret), "{secret} leaked: {printed}");
    }
    assert!(printed.contains("[REDACTED]"));
    assert!(
        printed.contains("vault_path"),
        "non-secret fields stay readable"
    );
}

#[test]
fn has_llm_key_counts_config_keys_and_conventional_env_vars() {
    let mut config = crate::config::HqConfig::default();
    let none = |_: &str| None;
    assert!(!config.has_llm_key_with(none));

    let env = |name: &str| (name == "OPENROUTER_API_KEY").then(|| "sk-or-test".to_string());
    assert!(config.has_llm_key_with(env));

    let blank = |name: &str| (name == "OPENROUTER_API_KEY").then(|| "  ".to_string());
    assert!(!config.has_llm_key_with(blank));

    config.anthropic_api_key = Some("sk-ant-test".into());
    assert!(config.has_llm_key_with(none));
}

#[test]
fn a_config_with_the_old_agent_host_heading_still_loads_and_the_new_one_wins() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.yaml");
    std::fs::write(&old, "herdr:\n  default_host: laptop\n  hosts:\n    laptop:\n      ssh: me@h\n").unwrap();
    let cfg = crate::HqConfig::load_from_path(&old).unwrap();
    assert_eq!(cfg.agent_host.default_host, "laptop");
    assert_eq!(cfg.agent_host.hosts["laptop"].ssh, "me@h");
    assert!(std::fs::read_to_string(&old).unwrap().starts_with("herdr:"), "the file on disk is left alone");

    let new = dir.path().join("new.yaml");
    std::fs::write(&new, "agent_host:\n  default_host: pc\n").unwrap();
    assert_eq!(crate::HqConfig::load_from_path(&new).unwrap().agent_host.default_host, "pc");

    let both = dir.path().join("both.yaml");
    std::fs::write(&both, "agent_host:\n  default_host: pc\nherdr:\n  default_host: laptop\n").unwrap();
    assert_eq!(crate::HqConfig::load_from_path(&both).unwrap().agent_host.default_host, "pc");
}
