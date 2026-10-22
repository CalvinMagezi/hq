//! Guards on the Tauri shell's configuration, read from the files themselves. The shell is not
//! built in this workspace (it needs a web view), so these tests are what keeps its security
//! settings from drifting: the CSP, which window may call which command, no plugins, no remote
//! content.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn shell() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn conf() -> Value {
    json(&shell().join("src-tauri/tauri.conf.json"))
}

/// Every key anywhere in a JSON document.
fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());
                keys(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| keys(v, out)),
        _ => {}
    }
}

fn quoted(text: &str) -> BTreeSet<String> {
    text.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_csp_is_strict_and_names_no_remote_origin() {
    let conf = conf();
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("a CSP string");
    let directive = |name: &str| -> Vec<&str> {
        csp.split(';')
            .map(str::trim)
            .find_map(|d| d.strip_prefix(name))
            .unwrap_or_else(|| panic!("the CSP has no {name}"))
            .split_whitespace()
            .collect()
    };
    assert_eq!(directive("default-src"), ["'self'"]);
    assert_eq!(directive("script-src"), ["'self'"]);
    assert_eq!(directive("style-src"), ["'self'"]);
    assert_eq!(directive("object-src"), ["'none'"]);
    assert_eq!(directive("base-uri"), ["'none'"]);
    assert_eq!(directive("form-action"), ["'none'"]);
    assert_eq!(directive("frame-ancestors"), ["'none'"]);
    // The only connection the page may open is Tauri's own command channel.
    assert_eq!(directive("connect-src"), ["ipc:", "http://ipc.localhost"]);
    for forbidden in [
        "unsafe-inline",
        "unsafe-eval",
        "https:",
        "wss:",
        "ws:",
        "*",
        "data:",
        "blob:",
    ] {
        assert!(
            !csp.contains(forbidden),
            "the CSP allows {forbidden}: {csp}"
        );
    }
}

#[test]
fn the_web_view_is_locked_down() {
    let conf = conf();
    assert_eq!(conf["app"]["withGlobalTauri"], false);
    assert_eq!(conf["app"]["security"]["freezePrototype"], true);
    assert_eq!(conf["app"]["macOSPrivateApi"], false);
    // Both windows are made in code, each with its navigation guard; none is declared here.
    assert_eq!(conf["app"]["windows"], serde_json::json!([]));
    assert_eq!(
        conf["app"]["security"]["capabilities"],
        serde_json::json!(["control"])
    );
    assert!(
        conf.get("plugins").is_none(),
        "no plugin, so no updater, shell or autostart"
    );
    let mut all = Vec::new();
    keys(&conf, &mut all);
    for key in all {
        let k = key.to_lowercase();
        assert!(
            !k.contains("remote")
                && !k.contains("devurl")
                && !k.contains("dangerous")
                && !k.contains("updater"),
            "unexpected key {key}"
        );
    }
}

#[test]
fn only_the_control_window_has_a_capability_and_it_grants_exactly_the_app_commands() {
    let dir = shell().join("src-tauri/capabilities");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let cap = json(&files[0]);
    assert_eq!(cap["identifier"], "control");
    assert_eq!(cap["windows"], serde_json::json!(["control"]));
    assert!(cap.get("remote").is_none() && cap.get("webviews").is_none());

    let granted: BTreeSet<String> = cap["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_owned())
        .collect();
    let app_permissions: BTreeSet<String> = granted
        .iter()
        .filter_map(|p| p.strip_prefix("allow-"))
        .map(|c| c.replace('-', "_"))
        .collect();
    let others: BTreeSet<&str> = granted
        .iter()
        .filter(|p| !p.starts_with("allow-"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        others,
        BTreeSet::from(["core:event:allow-listen", "core:event:allow-unlisten"])
    );

    // The build manifest and the handler list say the same thing as the capability.
    let build = read(&shell().join("src-tauri/build.rs"));
    let manifest = build
        .split("const COMMANDS")
        .nth(1)
        .and_then(|rest| rest.split("];").next())
        .expect("a COMMANDS list in build.rs");
    assert_eq!(
        quoted(manifest),
        app_permissions,
        "build.rs and the capability disagree"
    );
    let commands = read(&shell().join("src-tauri/src/commands.rs"));
    let handler = commands
        .split("generate_handler![")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("a generate_handler! list");
    let in_handler: BTreeSet<String> = handler
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        in_handler, app_permissions,
        "the handler and the capability disagree"
    );
}

#[test]
fn the_shell_has_no_plugin_and_so_no_login_item_shell_access_or_updater() {
    let manifest = read(&shell().join("src-tauri/Cargo.toml"));
    for dep in manifest
        .lines()
        .filter(|l| l.contains('=') && !l.trim_start().starts_with('#'))
    {
        assert!(!dep.trim_start().starts_with("tauri-plugin"), "{dep}");
    }
    assert!(!manifest.contains("autostart") && !manifest.contains("updater"));
    // And no code that writes a launch agent.
    let sources = [
        read(&shell().join("src-tauri/src/main.rs")),
        read(&shell().join("src-tauri/src/commands.rs")),
    ]
    .join("\n");
    for needle in ["LaunchAgents", "SMAppService", "launchctl", "Login Items"] {
        assert!(!sources.contains(needle), "{needle}");
    }
}

#[test]
fn both_windows_are_made_in_code_with_a_navigation_guard_and_no_new_windows() {
    let main = read(&shell().join("src-tauri/src/main.rs"));
    assert_eq!(main.matches("WebviewWindowBuilder::new").count(), 2);
    assert_eq!(main.matches(".on_navigation(").count(), 2);
    assert_eq!(main.matches("NewWindowResponse::Deny").count(), 2);
    assert!(main.contains("app_page_allowed") && main.contains("main_page_allowed"));
}

#[test]
fn the_control_window_loads_nothing_from_outside() {
    let ui = shell().join("ui");
    for name in ["index.html", "style.css"] {
        let text = read(&ui.join(name));
        assert!(
            !text.contains("http://") && !text.contains("https://"),
            "{name} names a remote URL"
        );
        assert!(!text.contains("@import"), "{name}");
    }
    let html = read(&ui.join("index.html"));
    assert!(
        !html.contains(" style="),
        "an inline style would need a CSP exception"
    );
    for tag in html.split("<script").skip(1) {
        let head = tag.split('>').next().unwrap();
        assert!(
            head.contains("src=\"main.js\""),
            "an inline or foreign script: <script{head}>"
        );
    }
    let ts = read(&ui.join("src/main.ts"));
    assert!(
        !ts.contains("innerHTML") && !ts.contains("insertAdjacentHTML") && !ts.contains("eval(")
    );
    assert!(!ts.contains("fetch(") && !ts.contains("XMLHttpRequest") && !ts.contains("WebSocket"));
}

#[test]
fn the_app_asks_macos_for_the_local_network_with_a_reason() {
    let plist = read(&shell().join("src-tauri/Info.plist"));
    assert!(plist.contains("NSLocalNetworkUsageDescription"));
}
