use anyhow::Result;
use hq_core::config::HqConfig;

/// Show or edit HQ configuration.
pub async fn run(config: &HqConfig, key: Option<&str>, value: Option<&str>) -> Result<()> {
    if let (Some(key), Some(value)) = (key, value) {
        // Set a config value
        set_config(key, value)?;
        println!("Set {} = {}", key, value);
        return Ok(());
    }

    if let Some(key) = key {
        // Show a specific key
        let val = get_config_value(config, key);
        println!("{}: {}", key, val);
        return Ok(());
    }

    // Show all config
    println!("Agent-HQ Configuration");
    println!("======================");
    println!();
    println!("Config file: {}", HqConfig::config_read_path().display());
    println!();
    println!("vault_path:        {}", config.vault_path.display());
    println!("default_model:     {}", config.default_model);
    println!("ws_port:           {}", config.ws_port);
    println!(
        "openrouter_api_key: {}",
        mask_key(&config.openrouter_api_key)
    );
    println!(
        "anthropic_api_key:  {}",
        mask_key(&config.anthropic_api_key)
    );
    println!(
        "google_ai_api_key:  {}",
        mask_key(&config.google_ai_api_key)
    );
    println!();
    print_backends(config);
    println!();
    println!("Agent:");
    println!("  name:                  {}", config.agent.name);
    println!();
    println!("Relay:");
    println!("  discord_enabled:       {}", config.relay.discord_enabled);
    println!("  telegram_enabled:      {}", config.relay.telegram_enabled);
    println!(
        "  discord_token:         {}",
        mask_key(&config.relay.discord_token)
    );
    println!(
        "  telegram_token:        {}",
        mask_key(&config.relay.telegram_token)
    );
    println!();
    println!("Daemon:");
    println!(
        "  embedding_batch_size:  {}",
        config.daemon.embedding_batch_size
    );
    println!(
        "  embedding_interval:    {}s",
        config.daemon.embedding_interval_secs
    );

    Ok(())
}

fn print_backends(config: &HqConfig) {
    let backends = &config.backends;
    println!("Backends (versioned provider chain):");
    if !backends.is_configured() {
        println!("  (not configured — using legacy providers / *_api_key fields)");
        return;
    }
    println!("  schema_version:  {}", backends.version);
    println!("  primary:         {}", backends.primary);
    if backends.fallbacks.is_empty() {
        println!("  fallbacks:       (none)");
    } else {
        println!("  fallbacks:       {}", backends.fallbacks.join(" -> "));
    }
    println!("  entries:");
    for entry in &backends.backends {
        let cred = match &entry.credential_env {
            Some(env) => {
                let present = std::env::var(env).map(|v| !v.is_empty()).unwrap_or(false);
                format!("{env} [{}]", if present { "set" } else { "unset" })
            }
            None => "(none)".to_string(),
        };
        let endpoint = entry
            .resolved_endpoint()
            .unwrap_or_else(|| "(n/a)".to_string());
        println!(
            "    - {:<16} kind={:?} enabled={} endpoint={} credential={}",
            entry.name, entry.kind, entry.enabled, endpoint, cred
        );
    }
    match backends.validate() {
        Ok(()) => println!("  validation:      ok"),
        Err(problems) => {
            println!("  validation:      {} problem(s)", problems.len());
            for p in problems {
                println!("    ! {p}");
            }
        }
    }
}

fn mask_key(key: &Option<String>) -> String {
    match key {
        Some(k) if k.len() > 8 => format!("{}...", &k[..8]),
        Some(k) if !k.is_empty() => format!("{}...", k),
        _ => "(not set)".to_string(),
    }
}

fn get_config_value(config: &HqConfig, key: &str) -> String {
    match key {
        "vault_path" => config.vault_path.display().to_string(),
        "default_model" => config.default_model.clone(),
        "ws_port" => config.ws_port.to_string(),
        "openrouter_api_key" => mask_key(&config.openrouter_api_key),
        "anthropic_api_key" => mask_key(&config.anthropic_api_key),
        "google_ai_api_key" => mask_key(&config.google_ai_api_key),
        "backends.primary" => config.backends.primary.clone(),
        "backends.version" => config.backends.version.to_string(),
        "agent.name" => config.agent.name.clone(),
        "discord_enabled" => config.relay.discord_enabled.to_string(),
        "telegram_enabled" => config.relay.telegram_enabled.to_string(),
        _ => format!("(unknown key: {})", key),
    }
}

fn set_config(key: &str, value: &str) -> Result<()> {
    let config_path = HqConfig::config_file_path();
    let mut content = if config_path.exists() {
        std::fs::read_to_string(&config_path)?
    } else {
        String::new()
    };

    // Simple YAML key-value replacement or append
    let pattern = format!("{}: ", key);
    let new_line = format!("{}: \"{}\"", key, value);

    if let Some(pos) = content.find(&pattern) {
        // Replace existing line
        let line_end = content[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(content.len());
        content.replace_range(pos..line_end, &new_line);
    } else {
        // Append
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&new_line);
        content.push('\n');
    }

    if let Some(parent) = config_path.parent() {
        hq_core::fs_private::create_private_dir_all(parent)?;
    }
    hq_core::fs_private::write_private(&config_path, content)?;

    Ok(())
}
