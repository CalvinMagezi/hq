use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;

pub async fn run_collection_tip(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.collection_tip {
        return Ok(());
    }
    hq_tools::collection_tip::run(db, chrono::Utc::now())?;
    Ok(())
}
