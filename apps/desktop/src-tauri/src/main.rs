//! The macOS app: a control window of its own (bundled files, strict CSP, the only window with
//! access to the commands below) and a second window that shows HQ's web app through the app's
//! loopback proxy, with no way to call the app and nowhere to navigate but that proxy.
//! All logic lives in `hq-desktop`; this file only turns its calls into commands and windows.

mod commands;

use hq_core::config::HqConfig;
use hq_desktop::{Desktop, DesktopInit};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tauri::{
    Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, webview::NewWindowResponse,
};

// Where the app is in closing, so a second quit never cuts the first one short.
const RUNNING: u8 = 0;
const CLOSING: u8 = 1;
const CLOSED: u8 = 2;

pub const CONTROL: &str = "control";
pub const MAIN: &str = "hq";
const CONTROL_SIZE: (f64, f64) = (940.0, 700.0);
const CONTROL_MIN: (f64, f64) = (640.0, 480.0);

/// What the control window reads once at start: a problem found while opening the app.
pub struct Startup {
    pub warning: Option<String>,
}

/// Tells the person why the app cannot open. A double-clicked app has no terminal to print to.
fn fatal(message: &str) -> ! {
    eprintln!("{message}");
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!("display alert \"HQ could not start\" message \"{escaped}\""),
        ])
        .status();
    std::process::exit(1);
}

fn open_desktop() -> (Arc<Desktop>, Startup) {
    let (config, warning) = match HqConfig::load() {
        Ok(config) => (config, None),
        Err(e) => (
            HqConfig::default(),
            Some(format!(
                "Could not read the HQ config, using defaults: {e:#}"
            )),
        ),
    };
    let init = match DesktopInit::from_environment(config) {
        Ok(init) => init,
        Err(e) => fatal(&format!("{e:#}")),
    };
    match Desktop::open(init) {
        Ok(desktop) => (Arc::new(desktop), Startup { warning }),
        Err(e) => fatal(&format!("{e:#}")),
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,hq_desktop=info".into()),
        )
        .init();
    let (desktop, startup) = open_desktop();
    let for_setup = desktop.clone();
    let phase = Arc::new(AtomicU8::new(RUNNING));
    let phase_for_setup = phase.clone();
    let app = tauri::Builder::default()
        .manage(desktop.clone())
        .manage(startup)
        .invoke_handler(commands::handler())
        .setup(move |app| {
            WebviewWindowBuilder::new(app, CONTROL, WebviewUrl::App("index.html".into()))
                .title("HQ")
                .inner_size(CONTROL_SIZE.0, CONTROL_SIZE.1)
                .min_inner_size(CONTROL_MIN.0, CONTROL_MIN.1)
                .on_navigation(|url| hq_desktop::window::app_page_allowed(url.as_str()))
                .on_new_window(|_, _| NewWindowResponse::Deny)
                .build()?;
            let handle = app.handle().clone();
            let mut changed = for_setup.pairing.subscribe();
            tauri::async_runtime::spawn(async move {
                while changed.changed().await.is_ok() {
                    let _ = handle.emit_to(CONTROL, "state-changed", ());
                }
            });
            let launch = for_setup.clone();
            tauri::async_runtime::spawn(async move { launch.on_launch().await });
            // A plain kill (SIGTERM, ctrl-c) closes what the app opened too.
            let (handle, desktop, phase) = (
                app.handle().clone(),
                for_setup.clone(),
                phase_for_setup.clone(),
            );
            tauri::async_runtime::spawn(async move {
                hq_desktop::desktop::wait_for_termination().await;
                close_then_exit(handle, desktop, phase).await;
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("could not build the app");

    app.run(move |app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            // Close what the app opened, including a host it started, before the process ends.
            // Until that is done every quit request is held, so a second one cannot skip it.
            if phase.load(Ordering::SeqCst) != CLOSED {
                api.prevent_exit();
                tauri::async_runtime::spawn(close_then_exit(
                    app.clone(),
                    desktop.clone(),
                    phase.clone(),
                ));
            }
        }
    });
}

/// Closes everything once, then ends the app. Later calls while it runs do nothing.
async fn close_then_exit(app: tauri::AppHandle, desktop: Arc<Desktop>, phase: Arc<AtomicU8>) {
    if phase
        .compare_exchange(RUNNING, CLOSING, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        desktop.shutdown().await;
        phase.store(CLOSED, Ordering::SeqCst);
        app.exit(0);
    }
}

/// Shows HQ's web app, or brings its window forward.
pub async fn show_main(app: &tauri::AppHandle, desktop: &Desktop) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(MAIN) {
        return window.set_focus().map_err(|e| e.to_string());
    }
    let url = desktop
        .main_window_url()
        .await
        .map_err(|e| format!("{e:#}"))?;
    let origin = desktop.proxy_origin().await.map_err(|e| format!("{e:#}"))?;
    let parsed: tauri::Url = url
        .parse()
        .map_err(|_| "the proxy address is not a URL".to_string())?;
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(parsed))
        .title("HQ")
        .inner_size(1200.0, 800.0)
        .on_navigation(move |url| hq_desktop::window::main_page_allowed(url.as_str(), &origin))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
