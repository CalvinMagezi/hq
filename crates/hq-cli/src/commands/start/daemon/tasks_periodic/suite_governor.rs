use anyhow::Result;
use hq_core::config::HqConfig;
use std::path::Path;
use tracing::{info, warn};

/// Tidies the HQ Suite's storage and keeps it inside `suite.budget_mb`. Off with `suite.governor:
/// false`. Without the `suite` feature there is nothing to govern.
#[cfg(feature = "suite")]
pub async fn run_suite_governor(vault_path: &Path, config: &HqConfig) -> Result<()> {
    use hq_suite_adapter::governor;
    if !config.suite.governor {
        return Ok(());
    }
    let settings = governor::settings(config.suite.budget_mb, config.suite.cold_window_days);
    let vault = vault_path.to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || governor::run(&vault, &settings)).await?;
    match outcome {
        Ok(o) => {
            let text = governor::outcome_text(&o);
            if !text.is_empty() {
                info!("suite-governor: {text}");
            }
        }
        Err(e) => warn!("suite-governor: {e:#}"),
    }
    Ok(())
}

#[cfg(not(feature = "suite"))]
pub async fn run_suite_governor(_vault_path: &Path, _config: &HqConfig) -> Result<()> {
    Ok(())
}
