//! Daemon utility functions — time helpers, flag files, directory management.

use std::path::Path;

/// Ensure a directory exists, creating it if needed.
pub fn ensure_dir(path: &Path) {
    if !path.exists() {
        let _ = std::fs::create_dir_all(path);
    }
}

/// Check if a time-gated task has already run today by looking for a flag file.
pub fn has_run_today(vault_path: &Path, task_name: &str) -> bool {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let flag_dir = vault_path.join("_system").join(".daemon-flags");
    let flag_file = flag_dir.join(format!("{task_name}-{today}"));
    flag_file.exists()
}

/// Mark a time-gated task as having run today.
pub fn mark_run_today(vault_path: &Path, task_name: &str) {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let flag_dir = vault_path.join("_system").join(".daemon-flags");
    ensure_dir(&flag_dir);
    let flag_file = flag_dir.join(format!("{task_name}-{today}"));
    let _ = std::fs::write(&flag_file, &today);
    // Clean up old flags (older than 3 days)
    if let Ok(entries) = std::fs::read_dir(&flag_dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy();
            if fname_str.starts_with(task_name)
                && !fname_str.ends_with(&today)
                && let Some(date_part) = fname_str.rsplit('-').collect::<Vec<_>>().get(..3)
            {
                let file_date = date_part
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>()
                    .join("-");
                if let Ok(fd) = chrono::NaiveDate::parse_from_str(&file_date, "%Y-%m-%d") {
                    let today_date = chrono::Utc::now().date_naive();
                    if today_date.signed_duration_since(fd).num_days() > 3 {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

/// Owner Telegram chat for proactive notifications (never guest last-active).
pub fn telegram_notification_chat_id(vault_path: &Path) -> Option<i64> {
    let relay = hq_core::config::HqConfig::load()
        .map(|c| c.relay)
        .unwrap_or_default();
    hq_core::telegram_access::notification_chat_id(vault_path, &relay)
}
