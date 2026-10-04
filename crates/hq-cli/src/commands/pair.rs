use anyhow::{Result, bail};
use hq_core::config::HqConfig;
use hq_core::pairing::{self, PairPlatform};
use std::path::Path;

pub fn parse_platform(name: &str) -> Result<PairPlatform> {
    match name.to_ascii_lowercase().as_str() {
        "telegram" => Ok(PairPlatform::Telegram),
        "discord" => Ok(PairPlatform::Discord),
        other => bail!("unknown platform '{other}' (use telegram or discord)"),
    }
}

/// The message the new owner sends to the bot, per platform.
fn pair_instruction(platform: PairPlatform, code: &str) -> String {
    match platform {
        PairPlatform::Telegram => format!("/pair {code}"),
        PairPlatform::Discord => format!("!pair {code}  (send it as a direct message)"),
    }
}

/// Prints a fresh one-time pairing code and how to use it.
pub fn issue(vault_path: &Path, platform: PairPlatform) -> Result<()> {
    let code = pairing::create_pairing_code(vault_path, platform, pairing::now_secs())
        .map_err(|e| anyhow::anyhow!("could not store the pairing code: {e:?}"))?;
    let minutes = pairing::PAIRING_TTL_SECS / 60;
    println!("Pairing code for {}: {code}", platform.as_str());
    println!("Send this to your bot from the account that should own HQ:");
    println!("    {}", pair_instruction(platform, &code));
    println!("The code works once and expires in {minutes} minutes.");
    println!("Only a hash is stored. Run this command again to replace the code.");
    Ok(())
}

pub fn run(config: &HqConfig, platform: &str) -> Result<()> {
    issue(&config.vault_path, parse_platform(platform)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_platforms_only() {
        assert_eq!(parse_platform("Telegram").unwrap(), PairPlatform::Telegram);
        assert_eq!(parse_platform("discord").unwrap(), PairPlatform::Discord);
        assert!(parse_platform("slack").is_err());
    }

    #[test]
    fn issue_stores_a_pending_code() {
        let dir = tempfile::tempdir().unwrap();
        issue(dir.path(), PairPlatform::Telegram).unwrap();
        assert!(pairing::has_pending_code(
            dir.path(),
            PairPlatform::Telegram,
            pairing::now_secs()
        ));
    }
}
