//! The commands the control window can call. Each is a thin call into `hq-desktop`; the list is
//! mirrored in `build.rs` and `capabilities/control.json`, and a test in `hq-desktop` checks that
//! the three agree.

use crate::Startup;
use hq_desktop::{
    Desktop, Revoked,
    agents::AgentsView,
    chooser::Picked,
    claude::HubView,
    desktop::{HelpersReport, HostLog, HubReport},
    helpers::TryOutcome,
    host::HostStatus,
    pairing::{PairingSnapshot, Submitted, TicketView},
    settings::Settings,
    share::ShareView,
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
        host_agents,
        host_log,
        host_unlock,
        pick_program,
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
        helpers_list,
        helpers_check,
        helper_try,
        helper_try_stop,
        helper_connect,
        helper_disconnect,
        helper_restore,
        helper_signin,
        share_status,
        share_set,
    ]
}

#[tauri::command]
fn startup_warning(startup: State<'_, Startup>) -> Option<String> {
    startup.warning.clone()
}

#[tauri::command]
async fn host_status(app: App<'_>) -> Reply<HubReport> {
    Ok(app.report().await)
}

#[tauri::command]
async fn host_start(app: App<'_>, restart: Option<bool>) -> Reply<HostStatus> {
    app.host
        .start_with(restart.unwrap_or(false))
        .await
        .map_err(shown)
}

#[tauri::command]
async fn host_stop(app: App<'_>) -> Reply<HostStatus> {
    app.host.stop().await.map_err(shown)
}

#[tauri::command]
async fn host_agents(app: App<'_>) -> Reply<AgentsView> {
    Ok(app.agents().await)
}

/// The last lines of the host's log with keys hidden, for the log sheet and Copy details.
#[tauri::command]
async fn host_log(app: App<'_>) -> Reply<HostLog> {
    Ok(app.host_log().await)
}

/// Lets coding helpers start again after a lockdown, by running `hq host unlock`.
#[tauri::command]
async fn host_unlock(app: App<'_>) -> Reply<()> {
    app.host_unlock().await.map_err(shown)
}

/// Chooses the `hq` program: an open panel, or a pasted path when `path` is given. The pick is
/// checked before it is used; the error is `refused`, `not_hq` or `not_found`, and `cancelled`
/// when the panel was closed.
#[tauri::command]
async fn pick_program(handle: AppHandle, app: App<'_>, path: Option<String>) -> Reply<Picked> {
    let path = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => crate::choose_program_file(&handle)
            .await
            .ok_or_else(|| "cancelled".to_string())?,
    };
    app.pick_program(path).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn settings_get(app: App<'_>) -> Settings {
    app.settings()
}

#[tauri::command]
fn settings_save(app: App<'_>, settings: Settings) -> Reply<()> {
    // What the app learned on its own is not the page's to set.
    let settings = settings.keeping_app_fields(&app.settings());
    app.save_settings(settings).map_err(shown)
}

#[tauri::command]
async fn open_main_window(handle: AppHandle, app: App<'_>, path: Option<String>) -> Reply<()> {
    crate::show_main(&handle, &app, path.as_deref().unwrap_or("/")).await
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
async fn pairing_revoke(app: App<'_>, id: String) -> Reply<Revoked> {
    app.revoke(&id).await.map_err(shown)
}

/// Every helper and where it stands. Reads state and runs nothing.
#[tauri::command]
async fn helpers_list(app: App<'_>) -> Reply<HelpersReport> {
    Ok(app.helpers_list().await)
}

/// Checks one helper (or all found) in a sandbox; `again` forgets earlier answers first.
#[tauri::command]
async fn helpers_check(
    app: App<'_>,
    id: Option<String>,
    again: Option<bool>,
) -> Reply<HelpersReport> {
    Ok(app.helpers_check(id, again.unwrap_or(false)).await)
}

/// Starts a helper in the Try it folder. `confirmed` is required for general-purpose helpers.
#[tauri::command]
async fn helper_try(
    app: App<'_>,
    id: String,
    confirmed: Option<bool>,
    signin: Option<bool>,
) -> Reply<TryOutcome> {
    app.helper_try(&id, confirmed.unwrap_or(false), signin.unwrap_or(false))
        .await
        .map_err(shown)
}

#[tauri::command]
async fn helper_try_stop(app: App<'_>) -> Reply<()> {
    app.helper_try_stop().await.map_err(shown)
}

/// Connects a helper to HQ on this computer, or (Claude only) to a paired hub.
#[tauri::command]
async fn helper_connect(app: App<'_>, id: String, hub: Option<String>) -> Reply<Option<HubView>> {
    app.helper_connect(&id, hub, true).await.map_err(shown)
}

#[tauri::command]
async fn helper_disconnect(
    app: App<'_>,
    id: String,
    hub: Option<String>,
) -> Reply<Option<HubView>> {
    app.helper_connect(&id, hub, false).await.map_err(shown)
}

/// Undoes the last disconnect of this helper, within ten minutes.
#[tauri::command]
fn helper_restore(app: App<'_>, id: String) -> Reply<()> {
    app.helper_restore(&id).map_err(shown)
}

/// Copies a helper's held sign-in link, or opens it when `open` and it is on the helper's own site.
#[tauri::command]
fn helper_signin(app: App<'_>, id: String, open: Option<bool>) -> Reply<()> {
    app.helper_signin(&id, open.unwrap_or(false)).map_err(shown)
}

#[tauri::command]
async fn share_status(app: App<'_>) -> Reply<ShareView> {
    Ok(app.share_status().await)
}

/// Lends this computer's model to its hubs, or stops at once.
#[tauri::command]
fn share_set(app: App<'_>, on: bool, model: Option<String>) -> Reply<()> {
    app.set_share(on, model).map_err(shown)
}
