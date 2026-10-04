use anyhow::Result;
use hq_core::config::HqConfig;
use hq_core::mailbox;

/// Days of mail kept in the inbox when `archive` is called without a cutoff.
const DEFAULT_ARCHIVE_DAYS: i64 = 7;

/// Inspect and maintain the inter-agent mailboxes under `_mailboxes/`.
pub async fn run(config: &HqConfig, sub: &str, args: &[String]) -> Result<()> {
    match sub {
        "list" | "ls" | "" => list(config),
        "archive" => archive(config, args),
        other => anyhow::bail!("unknown subcommand '{other}'. Try: list, archive"),
    }
}

fn list(config: &HqConfig) -> Result<()> {
    let base = config.vault_path.join(mailbox::MAILBOX_DIR);
    if !base.exists() {
        println!("No mailboxes yet.");
        return Ok(());
    }

    let mut rows: Vec<(String, usize)> = std::fs::read_dir(&base)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let count = mailbox::message_count(&config.vault_path, &name).unwrap_or(0);
            (name, count)
        })
        .collect();
    rows.sort();

    println!("Mailboxes");
    println!("{}\n", "=".repeat(40));
    for (name, count) in &rows {
        let reserved = if mailbox::RESERVED_MAILBOXES.contains(&name.as_str()) {
            "  (live consumer)"
        } else {
            ""
        };
        println!("  {name:<18} {count:>4} pending{reserved}");
    }
    Ok(())
}

fn archive(config: &HqConfig, args: &[String]) -> Result<()> {
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let days = parse_days(args)?;

    let moved = mailbox::archive_older_than(&config.vault_path, days, dry_run)?;
    if moved.is_empty() {
        println!("Nothing older than {days} days to archive.");
        return Ok(());
    }

    let total: usize = moved.iter().map(|(_, n)| n).sum();
    let verb = if dry_run { "Would archive" } else { "Archived" };
    println!("{verb} {total} messages older than {days} days:\n");
    for (name, count) in &moved {
        println!("  {name:<18} {count:>4}");
    }
    if dry_run {
        println!("\nRe-run without --dry-run to move them.");
    } else {
        println!(
            "\nMoved into each mailbox's {}/ subdirectory.",
            mailbox::ARCHIVE_DIR
        );
    }
    Ok(())
}

/// Accepts `--older-than-days N` or a bare positional number.
fn parse_days(args: &[String]) -> Result<i64> {
    let flagged = args
        .iter()
        .position(|a| a == "--older-than-days")
        .and_then(|i| args.get(i + 1));
    let raw = flagged.or_else(|| args.iter().find(|a| !a.starts_with('-')));

    match raw {
        None => Ok(DEFAULT_ARCHIVE_DAYS),
        Some(v) => v
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("expected a number of days, got '{v}'"))
            .and_then(|d| {
                if d < 0 {
                    anyhow::bail!("days must not be negative");
                }
                Ok(d)
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_default_when_unspecified() {
        assert_eq!(parse_days(&[]).unwrap(), DEFAULT_ARCHIVE_DAYS);
        assert_eq!(
            parse_days(&["--dry-run".to_string()]).unwrap(),
            DEFAULT_ARCHIVE_DAYS
        );
    }

    #[test]
    fn days_parse_from_flag_or_position() {
        let flag = vec!["--older-than-days".to_string(), "30".to_string()];
        assert_eq!(parse_days(&flag).unwrap(), 30);
        assert_eq!(parse_days(&["14".to_string()]).unwrap(), 14);
    }

    #[test]
    fn bad_days_are_rejected_rather_than_defaulted() {
        assert!(parse_days(&["soon".to_string()]).is_err());
        let negative = vec!["--older-than-days".to_string(), "-3".to_string()];
        assert!(parse_days(&negative).is_err());
    }
}
