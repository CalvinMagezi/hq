//! `hq focus`: the few lanes of work the owner chose to move forward. Everything else is parked.

use anyhow::{Result, bail};
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::{Database, focus_lanes};

#[derive(Subcommand)]
pub enum FocusAction {
    /// List the lanes
    List,
    /// Make a lane active (or bring a parked one back)
    Add { lane: String },
    /// Set a lane aside without forgetting it
    Park { lane: String },
    /// Forget a lane
    Remove { lane: String },
}

pub fn run(config: &HqConfig, action: FocusAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    match action {
        FocusAction::List => {
            let lanes = focus_lanes::list(&db)?;
            if lanes.is_empty() {
                println!(
                    "No lanes yet. `hq focus add <name>` (up to {}).",
                    focus_lanes::max_lanes(&db)
                );
            }
            for l in lanes {
                println!("{}{}", l.name, if l.parked { "  (parked)" } else { "" });
            }
        }
        FocusAction::Add { lane } => {
            focus_lanes::add(&db, &lane)?;
            println!("{} is active.", lane.trim());
        }
        FocusAction::Park { lane } => {
            if !focus_lanes::park(&db, &lane)? {
                bail!("no active lane named '{}'", lane.trim());
            }
            println!("{} is parked.", lane.trim());
        }
        FocusAction::Remove { lane } => {
            if !focus_lanes::remove(&db, &lane)? {
                bail!("no lane named '{}'", lane.trim());
            }
            println!("{} is removed.", lane.trim());
        }
    }
    Ok(())
}
