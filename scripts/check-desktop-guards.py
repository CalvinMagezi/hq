#!/usr/bin/env python3
"""Breaks each security guard of the desktop app in turn, runs the hq-desktop tests, and reports
whether a test failed. The file is always restored. Run from the repository root:

    python3 scripts/check-desktop-guards.py            # all guards
    python3 scripts/check-desktop-guards.py proxy Host  # only guards whose name contains a word

A guard whose break makes no test fail prints NOT CAUGHT and the script exits 1. Slow (a build
and a test run per guard), so it is a pre-release check, not part of CI. See
docs/security/DESKTOP.md, section 8.
"""
import subprocess, re, sys, json, os
ROOT = os.getcwd()
M = [
 ("digits compare forced true", "crates/hq-desktop/src/pairing.rs",
  "typed.as_bytes().ct_eq(expected.as_bytes()).into()", "{ let _ = (typed, expected); true }"),
 ("revoke through the database instead of the running bridge", "crates/hq-desktop/src/pairing.rs",
  "Some(bridge) => bridge.revoke(&id).await?,", "Some(_) => { let db = self.db()?; let key = id.to_string(); tokio::task::spawn_blocking(move || peers::revoke(&db, &key)).await?? }"),
 ("expected digits written into the request view", "crates/hq-desktop/src/pairing.rs",
  "        name: p.name.clone(),\n        joiner_short", "        name: format!(\"{} {}\", p.name, p.sas),\n        joiner_short"),
 ("unlimited digit tries", "crates/hq-desktop/src/pairing.rs",
  "p.tries_left = p.tries_left.saturating_sub(1);", "p.tries_left = p.tries_left.max(2);"),
 ("proxy Host check removed", "crates/hq-desktop/src/proxy.rs",
  "Some(h) if !is_own(h) => return Err(Refusal::WrongHost),", "Some(_) => {}"),
 ("non-loopback bind allowed", "crates/hq-desktop/src/proxy.rs",
  "if !self.listen.ip().is_loopback() {", "if false {"),
 ("non-loopback upstream allowed", "crates/hq-desktop/src/proxy.rs",
  "if !self.upstream.ip().is_loopback() {", "if false {"),
 ("absolute-form and CONNECT allowed", "crates/hq-desktop/src/proxy.rs",
  "    if method == Method::CONNECT {\n        return Err(Refusal::BadTarget);\n    }\n    // Absolute-form targets are how a client asks a proxy to fetch someone else's URL.\n    if uri.scheme().is_some() || uri.authority().is_some_and(|a| !is_own(a.as_str())) {\n        return Err(Refusal::BadTarget);\n    }\n", ""),
 ("Origin check removed", "crates/hq-desktop/src/proxy.rs",
  "        if !ok {\n            return Err(Refusal::CrossOrigin);\n        }", "        let _ = ok;"),
 ("cross-site fetch allowed", "crates/hq-desktop/src/proxy.rs",
  "if matches!(fetch_site, Some(\"cross-site\" | \"same-site\")) && !navigation {", "if false && matches!(fetch_site, Some(\"cross-site\" | \"same-site\")) && !navigation {"),
 ("machine endpoints served", "crates/hq-desktop/src/proxy.rs",
  "    if is_machine_path(uri.path()) {\n        return Err(Refusal::MachineEndpoint);\n    }\n", ""),
 ("proxy adds a credential", "crates/hq-desktop/src/proxy.rs",
  "    // HQ builds its CSP and its origin checks from `Host`, so the client's own is kept.", "    headers.insert(header::AUTHORIZATION, HeaderValue::from_static(\"Bearer injected\"));\n    // HQ builds its CSP and its origin checks from `Host`, so the client's own is kept."),
 ("any websocket path carried", "crates/hq-desktop/src/proxy.rs",
  "        if !is_ws_path(req.uri().path()) {\n            return StatusCode::NOT_FOUND.into_response();\n        }\n", ""),
 ("second CSP not added", "crates/hq-desktop/src/proxy.rs",
  "        out.append(header::CONTENT_SECURITY_POLICY, state.csp.clone());", "        let _ = &state.csp;"),
 ("CORS grants kept", "crates/hq-desktop/src/proxy.rs",
  "if !hop && !n.starts_with(\"access-control-\") {", "if !hop {"),
 ("web window origin compared as a prefix", "crates/hq-desktop/src/window.rs",
  "    let (Ok(url), Ok(origin)) = (Url::parse(url), Url::parse(origin)) else {\n        return false;\n    };", "    if url.starts_with(origin) {\n        return true;\n    }\n    let (Ok(url), Ok(origin)) = (Url::parse(url), Url::parse(origin)) else {\n        return false;\n    };"),
 ("stopping a host the app did not start is allowed", "crates/hq-desktop/src/host.rs",
  "                Err(HostError::NotOurs)", "                Ok(HostStatus::Stopped)"),
 ("starting over a host that is already running is allowed", "crates/hq-desktop/src/host.rs",
  "            return Err(HostError::AlreadyRunning);", "            let _ = HostError::AlreadyRunning;"),
 ("stop signals the child, not its group", "crates/hq-desktop/src/host.rs",
  "libc::killpg(pid as libc::pid_t, signal);", "libc::kill(pid as libc::pid_t, signal);"),
 ("unknown settings keys accepted", "crates/hq-desktop/src/settings.rs",
  "#[serde(default, deny_unknown_fields)]", "#[serde(default)]"),
 ("host starts at launch whatever the setting says", "crates/hq-desktop/src/desktop.rs",
  "if self.settings().start_host_on_launch\n            &&", "if true\n            &&"),
 ("CSP allows inline scripts", "apps/desktop/src-tauri/tauri.conf.json",
  "script-src 'self';", "script-src 'self' 'unsafe-inline';"),
 ("CSP allows any https connection", "apps/desktop/src-tauri/tauri.conf.json",
  "connect-src ipc: http://ipc.localhost;", "connect-src ipc: http://ipc.localhost https:;"),
 ("global Tauri object on", "apps/desktop/src-tauri/tauri.conf.json",
  "\"withGlobalTauri\": false", "\"withGlobalTauri\": true"),
 ("a remote URL granted app commands", "apps/desktop/src-tauri/capabilities/control.json",
  "\"windows\": [\n    \"control\"\n  ],", "\"windows\": [\n    \"control\"\n  ],\n  \"remote\": {\"urls\": [\"http://127.0.0.1:47474\"]},"),
 ("a command granted that is not in the manifest", "apps/desktop/src-tauri/capabilities/control.json",
  "\"allow-host-status\",", "\"allow-host-status\",\n    \"allow-host-nuke\","),
 ("an updater plugin added", "apps/desktop/src-tauri/Cargo.toml",
  "tracing = \"0.1\"", "tracing = \"0.1\"\ntauri-plugin-autostart = \"2\""),
 ("innerHTML in the control window script", "apps/desktop/ui/src/main.ts",
  "node.textContent = text", "node.innerHTML = text"),
 ("a window without a navigation guard", "apps/desktop/src-tauri/src/main.rs",
  "        .on_navigation(move |url| hq_desktop::window::main_page_allowed(url.as_str(), &origin))\n", ""),
]
only = sys.argv[1:] 
rows = []
for name, path, old, new in M:
    if only and not any(o in name for o in only): continue
    full = os.path.join(ROOT, path)
    orig = open(full).read()
    if old not in orig:
        rows.append((name, "SKIPPED: pattern not found", []))
        print("SKIPPED", name, flush=True); continue
    try:
        open(full, "w").write(orig.replace(old, new, 1))
        r = subprocess.run(["cargo","test","-p","hq-desktop","--locked","--no-fail-fast"],
                           capture_output=True, text=True, cwd=ROOT, timeout=900)
        out = r.stdout + r.stderr
        failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
        broke = "error[" in out or "could not compile" in out
        status = "COMPILE ERROR" if broke else ("caught" if failed else "NOT CAUGHT")
        rows.append((name, status, failed))
        print(status, "|", name, "|", ", ".join(failed)[:200], flush=True)
    finally:
        open(full, "w").write(orig)
missed = [n for n, status, _ in rows if status != "caught"]
print(f"{len(rows) - len(missed)} of {len(rows)} caught" + (f"; not caught: {missed}" if missed else ""))
sys.exit(1 if missed else 0)
