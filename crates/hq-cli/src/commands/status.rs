use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use crate::render as ansi;
use crate::render::Theme;
use hq_vault::VaultClient;

pub async fn run(config: &HqConfig) -> Result<()> {
    let theme = Theme::dark();

    println!("{}", ansi::bold("Agent-HQ Status", &theme.primary));
    println!("{}\n", ansi::colored(&"=".repeat(15), &theme.border));

    // Vault info
    let vault = VaultClient::new(config.vault_path.clone())?;
    let (note_count, db_size) = vault.get_stats()?;

    println!("Vault:     {}", config.vault_path.display());
    println!("Notes:     {}", note_count);

    // Database info
    let db_path = config.db_path();
    if db_path.exists() {
        let db = Database::open(&db_path)?;
        let indexed = db.with_conn(hq_db::search::indexed_count)?;
        println!(
            "DB:        {} ({} indexed notes)",
            db_path.display(),
            indexed
        );
        println!("DB size:   {:.1} MB", db_size as f64 / 1_048_576.0);
    } else {
        println!("DB:        not initialized (run `hq setup`)");
    }

    // Config
    println!();
    println!("Model:     {}", config.default_model);
    println!("WS port:   {}", config.ws_port);
    println!(
        "OpenRouter: {}",
        if config.openrouter_api_key.is_some() {
            "configured"
        } else {
            "not set"
        }
    );
    println!(
        "Discord:   {}",
        if config.relay.discord_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "Telegram:  {}",
        if config.relay.telegram_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );

    Ok(())
}
