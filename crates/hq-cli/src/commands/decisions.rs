//! `hq decisions` prints what the structured-decision gates have been doing.

use anyhow::Result;
use hq_core::config::HqConfig;

pub fn run(config: &HqConfig, days: u32, site: Option<&str>) -> Result<()> {
    print!("{}", hq_llm::decision_report::report(&config.vault_path, days, site));
    Ok(())
}
