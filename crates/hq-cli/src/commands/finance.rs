//! `hq finance`: turn a message export into ledger drafts, entirely on this machine. Argument
//! handling only; the work is in `hq-finance`. Output is counts, never message text.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_finance::{export::drafts_from_export, ledger, validate};

/// Where the ledger lives. Under `_data/`, which agent read tools refuse and the vault listing skips.
const LEDGER_REL: &str = "_data/finance/ledger.md";

#[derive(Subcommand)]
pub enum FinanceCommand {
    /// Add transactions from a message export to the ledger as drafts
    ///
    /// The export is a JSON file you chose: a list of { "sender", "date", "body" }. Nothing reads
    /// your inbox, and nothing leaves this machine. Rows already in the ledger are never changed,
    /// so importing the same file twice adds nothing.
    Import {
        /// The export file
        messages: PathBuf,
        /// Show what would be added and write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Check an export against the balances the messages report; prints counts only
    Check {
        /// The export file
        messages: PathBuf,
    },
}

fn read_drafts(messages: &Path) -> Result<Vec<hq_finance::parse::Draft>> {
    let json = std::fs::read_to_string(messages)
        .with_context(|| format!("{} cannot be read", messages.display()))?;
    drafts_from_export(&json).context("the export is not a list of { sender, date, body }")
}

pub fn run(config: &HqConfig, command: FinanceCommand) -> Result<()> {
    match command {
        FinanceCommand::Import { messages, dry_run } => import(config, &messages, dry_run),
        FinanceCommand::Check { messages } => check(&messages),
    }
}

fn import(config: &HqConfig, messages: &Path, dry_run: bool) -> Result<()> {
    let drafts = read_drafts(messages)?;
    let path = config.vault_path.join(LEDGER_REL);
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => bail!("the ledger cannot be read: {e}"),
    };
    let (note, added) = ledger::import(existing.as_deref(), &drafts);
    println!("{} transaction messages found, {added} new", drafts.len());
    if dry_run {
        println!("dry run: nothing written");
        return Ok(());
    }
    if added == 0 {
        println!("the ledger is up to date");
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        hq_core::fs_private::create_private_dir_all(parent)?;
    }
    hq_core::fs_private::write_private(&path, note)?;
    println!("written to {LEDGER_REL} in the vault; open it in Sheets and confirm each row");
    Ok(())
}

fn check(messages: &Path) -> Result<()> {
    let report = validate::replay(&read_drafts(messages)?);
    println!("{}", serde_json::to_string_pretty(&report)?);
    match report.explained_share() {
        Some(s) => println!(
            "{:.0}% of balance gaps are explained by the parsed amounts",
            s * 100.0
        ),
        None => println!("no two messages reported a balance, so nothing could be checked"),
    }
    Ok(())
}
