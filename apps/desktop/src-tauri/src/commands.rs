//! The commands the control window can call. Each is a thin call into `hq-desktop`; the list is
//! mirrored in `build.rs` and `capabilities/control.json`, and a test in `hq-desktop` checks that
//! the three agree.

use crate::Startup;
use hq_desktop::{
    Desktop,
    claude::HubView,
    host::HostStatus,
    pairing::{PairingSnapshot, Submitted, TicketView},
    settings::Settings,
};
use std::sync::Arc;
use tauri::{AppHandle, State};

type App<'a> = State<'a, Arc<Desktop>>;
type Reply<T> = Result<T, String>;

fn shown(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        startup_warning,
        host_status,
        host_start,
        host_stop,
        settings_get,
        settings_save,
        open_main_window,
        open_in_browser,
        pairing_start,
        pairing_stop,
        pairing_begin,
        pairing_cancel,
        pairing_snapshot,
        pairing_submit,
        pairing_decline,
        pairing_revoke,
        claude_status,
        claude_change,
    ]
}

#[tauri::command]
fn startup_warning(startup: State<'_, Startup>) -> Option<String> {
    startup.warning.clone()
}

#[tauri::command]
async fn host_status(app: App<'_>) -> Reply<HostStatus> {
    Ok(app.host.status().await)
}

#[tauri::command]
async fn host_start(app: App<'_>) -> Reply<HostStatus> {
    app.host.start().await.map_err(shown)
}

#[tauri::command]
async fn host_stop(app: App<'_>) -> Reply<HostStatus> {
    app.host.stop().await.map_err(shown)
}

#[tauri::command]
fn settings_get(app: App<'_>) -> Settings {
    app.settings()
}

#[tauri::command]
fn settings_save(app: App<'_>, settings: Settings) -> Reply<()> {
    app.save_settings(settings).map_err(shown)
}

#[tauri::command]
async fn open_main_window(handle: AppHandle, app: App<'_>) -> Reply<()> {
    crate::show_main(&handle, &app).await
}

/// Hands an address on the app's own proxy to the system browser, so the consent page can be
/// signed in and approved there. Any other address is refused.
#[tauri::command]
async fn open_in_browser(app: App<'_>, url: String) -> Reply<()> {
    let origin = app.proxy_origin().await.map_err(shown)?;
    let target = hq_desktop::window::browser_target(&url, &origin)
        .ok_or_else(|| "only this app's own address can be opened".to_string())?;
    let status = std::process::Command::new("/usr/bin/open")
        .arg(target)
        .status()
        .map_err(shown)?;
    if status.success() {
        Ok(())
    } else {
        Err("the system could not open the address".into())
    }
}

#[tauri::command]
async fn pairing_start(app: App<'_>) -> Reply<()> {
    app.pairing.start().await.map_err(shown)
}

#[tauri::command]
async fn pairing_stop(app: App<'_>) -> Reply<()> {
    app.pairing.stop().await;
    Ok(())
}

#[tauri::command]
async fn pairing_begin(app: App<'_>) -> Reply<TicketView> {
    app.pairing.begin().await.map_err(shown)
}

#[tauri::command]
fn pairing_cancel(app: App<'_>) {
    app.pairing.cancel_ticket();
}

#[tauri::command]
async fn pairing_snapshot(app: App<'_>) -> Reply<PairingSnapshot> {
    Ok(app.pairing.snapshot().await)
}

#[tauri::command]
fn pairing_submit(app: App<'_>, id: u64, typed: String) -> Submitted {
    app.pairing.submit_digits(id, &typed)
}

#[tauri::command]
fn pairing_decline(app: App<'_>, id: u64) -> bool {
    app.pairing.decline(id)
}

#[tauri::command]
async fn pairing_revoke(app: App<'_>, id: String) -> Reply<bool> {
    app.revoke(&id).await.map_err(shown)
}

#[tauri::command]
async fn claude_status(app: App<'_>) -> Reply<Vec<HubView>> {
    app.claude_status().await.map_err(shown)
}

/// Adds a paired hub to Claude Code and Claude Desktop, or removes it.
#[tauri::command]
async fn claude_change(app: App<'_>, hub: String, add: bool) -> Reply<HubView> {
    app.claude_change(hub, add).await.map_err(shown)
}
