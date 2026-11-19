//! `hq decide`: put a question to yourself with a default and a deadline, and answer it.

use anyhow::{Result, bail};
use chrono::{Duration, Utc};
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::owner_decisions::{self, NewDecision};

#[derive(Subcommand)]
pub enum DecideAction {
    /// Raise a decision. Reversible ones take the default at the deadline; others are held.
    Create {
        question: String,
        /// Comma-separated options, at least two
        #[arg(long, value_delimiter = ',')]
        options: Vec<String>,
        /// The recommended option
        #[arg(long)]
        default: String,
        /// Hours until the deadline
        #[arg(long, default_value_t = 24)]
        hours: i64,
        /// Going with the default can be undone
        #[arg(long)]
        reversible: bool,
    },
    /// Open decisions
    List,
    /// Answer one
    Answer { id: String, option: String },
}

pub fn run(config: &HqConfig, action: DecideAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    match action {
        DecideAction::Create {
            question,
            options,
            default,
            hours,
            reversible,
        } => {
            if hours <= 0 {
                bail!("--hours must be more than zero");
            }
            let d = owner_decisions::create(
                &db,
                &NewDecision {
                    question: &question,
                    options: &options,
                    default_option: &default,
                    deadline_at: Utc::now() + Duration::hours(hours),
                    reversible,
                },
            )?;
            println!(
                "Decision {} raised. Answer with: hq decide answer {} <option>",
                d.id, d.id
            );
        }
        DecideAction::List => {
            for d in owner_decisions::list_open(&db)? {
                println!(
                    "{}  {}  [{}]  default {}  due {}",
                    d.id,
                    d.question,
                    d.options.join("/"),
                    d.default_option,
                    d.deadline_at.format("%Y-%m-%d %H:%M")
                );
            }
        }
        DecideAction::Answer { id, option } => {
            let d = owner_decisions::answer(&db, &id, &option)?;
            println!(
                "Decision {} answered: {}.",
                d.id,
                d.chosen.unwrap_or_default()
            );
        }
    }
    Ok(())
}
