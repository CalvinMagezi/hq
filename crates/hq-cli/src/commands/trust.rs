//! `hq trust list|add|remove`: the domains whose fetches by an agent never ask for approval.
//! Only the owner runs this; no agent tool writes the list. A trusted site still taints the
//! session (`docs/security/PROMPT_INJECTION.md`).

use anyhow::Result;
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::{Database, fetch_sites};

#[derive(Subcommand)]
pub enum TrustAction {
    /// Show the trusted domains and the sites earned from approvals
    List,
    /// Trust a domain and its subdomains, like `example.com`
    Add { domain: String },
    /// Stop trusting a domain or earned site, so HQ asks again
    Remove { domain: String },
}

pub fn run(config: &HqConfig, action: TrustAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    match action {
        TrustAction::List => {
            let (earned, domains) =
                db.with_conn(|c| Ok((fetch_sites::trusted(c)?, fetch_sites::trusted_domains(c)?)))?;
            println!("Trusted domains (you added):");
            domains.iter().for_each(|d| println!("  {d}"));
            println!("Sites earned from approvals:");
            earned.iter().for_each(|s| println!("  {s}"));
        }
        TrustAction::Add { domain } => {
            let stored = db.with_conn(|c| fetch_sites::add_trusted_domain(c, &domain))?;
            println!(
                "Trusting {stored} and its subdomains. Pages read there still count as untrusted text."
            );
        }
        TrustAction::Remove { domain } => {
            db.with_conn(|c| fetch_sites::forget(c, &domain))?;
            println!("Removed {domain}. HQ asks again.");
        }
    }
    Ok(())
}
