// Every command the control window may call is listed here, and each one needs a permission in
// capabilities/control.json. With this manifest Tauri refuses any command that is not granted to
// the window asking, whatever page that window shows.
const COMMANDS: &[&str] = &[
    "startup_warning",
    "host_status",
    "host_start",
    "host_stop",
    "host_agents",
    "host_log",
    "host_unlock",
    "pick_program",
    "settings_get",
    "settings_save",
    "open_main_window",
    "open_in_browser",
    "pairing_start",
    "pairing_stop",
    "pairing_begin",
    "pairing_cancel",
    "pairing_snapshot",
    "pairing_submit",
    "pairing_decline",
    "pairing_revoke",
    "helpers_list",
    "helpers_check",
    "helper_try",
    "helper_try_stop",
    "helper_connect",
    "helper_disconnect",
    "helper_restore",
    "helper_signin",
    "share_status",
    "share_set",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run the Tauri build step");
}
