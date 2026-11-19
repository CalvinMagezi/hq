use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

const SHELL: &str = r#"<html><head><script>boot()</script></head><body><script type="module" src="/assets/m.js"></script></body></html>"#;

fn app_with_vault(files: &[(&str, &str)]) -> (Router, tempfile::TempDir, tempfile::TempDir) {
    let vault = tempfile::TempDir::new().unwrap();
    for (name, body) in files {
        std::fs::write(vault.path().join(name), body).unwrap();
    }
    let web = tempfile::TempDir::new().unwrap();
    std::fs::write(web.path().join("index.html"), SHELL).unwrap();
    let state = WsState::new(vault.path().to_path_buf(), Some(web.path().to_path_buf()));
    (create_router(Arc::new(state)), vault, web)
}

async fn get(app: &Router, uri: &str) -> axum::response::Response {
    let req = Request::get(uri)
        .header("host", "127.0.0.1:5678")
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

#[tokio::test]
async fn every_response_carries_the_baseline_headers() {
    let (app, _v, _w) = app_with_vault(&[]);
    for uri in ["/", "/some/client/route", "/health", "/api/nope"] {
        let res = get(&app, uri).await;
        let h = res.headers();
        assert_eq!(h["x-content-type-options"], "nosniff", "{uri}");
        assert_eq!(h["referrer-policy"], "no-referrer", "{uri}");
        assert_eq!(h["x-frame-options"], "DENY", "{uri}");
        let csp = h["content-security-policy"].to_str().unwrap();
        assert!(
            csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"),
            "{uri}: {csp}"
        );
    }
}

#[tokio::test]
async fn the_served_shell_is_runnable_under_its_own_policy() {
    let (app, _v, _w) = app_with_vault(&[]);
    let res = get(&app, "/").await;
    let csp = res.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    let html = String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    for hash in security_headers::inline_script_hashes(&html) {
        assert!(
            csp.contains(&format!("'{hash}'")),
            "inline script not allowed: {hash}"
        );
    }
    let script_src = csp
        .split(';')
        .find(|d| d.trim().starts_with("script-src"))
        .unwrap();
    assert!(!script_src.contains("'unsafe-inline'"), "{script_src}");
}

/// Built-app tests need `bun run build` in apps/hq-web. CI sets
/// `HQ_REQUIRE_BUILT_APP=1` so a missing build fails instead of skipping.
fn built_app_missing(what: &str) {
    assert!(
        std::env::var_os("HQ_REQUIRE_BUILT_APP").is_none(),
        "HQ_REQUIRE_BUILT_APP is set but apps/hq-web is not built ({what})"
    );
    eprintln!("skipped: apps/hq-web is not built");
}

#[tokio::test]
async fn the_socket_host_of_the_request_is_allowed_in_connect_src() {
    let (app, _v, _w) = app_with_vault(&[]);
    let req = Request::get("/")
        .header("host", "hq.example.ts.net:8443")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let csp = res.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        csp.contains("ws://hq.example.ts.net:8443 wss://hq.example.ts.net:8443"),
        "{csp}"
    );
}

/// Reads the real build when it exists (`bun run build` in apps/hq-web).
#[test]
fn a_built_shell_only_loads_same_origin_assets_allowed_by_the_policy() {
    let dist =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/hq-web/dist/client");
    let Ok(bytes) = std::fs::read(dist.join("index.html")) else {
        built_app_missing("index.html");
        return;
    };
    let html = String::from_utf8_lossy(&bytes).into_owned();
    let csp = security_headers::content_security_policy(&dist);
    let csp = csp.to_str().unwrap();
    let hashes = security_headers::inline_script_hashes(&html);
    let inline_count = html
        .split("<script")
        .skip(1)
        .filter(|t| !t.split('>').next().unwrap_or("").contains("src="))
        .count();
    assert!(
        inline_count >= 1 && hashes.len() == inline_count,
        "{inline_count} inline scripts, {} hashes",
        hashes.len()
    );
    for hash in &hashes {
        assert!(
            csp.contains(&format!("'{hash}'")),
            "inline script missing from CSP"
        );
    }
    // The TanStack stream barrier holds raw NUL bytes; the hash must be of the parser's text.
    if html.contains('\u{FFFD}') || bytes.contains(&0) {
        let raw_hash = |t: &str| {
            use base64::Engine;
            use sha2::Digest;
            format!(
                "sha256-{}",
                base64::engine::general_purpose::STANDARD
                    .encode(sha2::Sha256::digest(t.as_bytes()))
            )
        };
        let raw_script = html
            .split("<script class=\"$tsr\" id=\"$tsr-stream-barrier\">")
            .nth(1)
            .and_then(|r| r.split("</script>").next());
        if let Some(raw) = raw_script.filter(|r| r.contains('\0')) {
            assert!(
                !csp.contains(&raw_hash(raw)),
                "the raw-byte hash must not be what is served"
            );
        }
    }
    for attr in ["src=\"", "href=\""] {
        for part in html.split(attr).skip(1) {
            let url = part.split('"').next().unwrap();
            // A link to a place on this page (the skip link) loads nothing.
            let in_page = attr == "href=\"" && url.starts_with('#');
            assert!(
                in_page || (url.starts_with('/') && !url.starts_with("//")),
                "cross-origin or inline load: {url}"
            );
        }
    }
    assert!(
        !html.contains("onclick="),
        "inline event handlers are blocked by the policy"
    );
}

/// `nosniff` makes a browser refuse a worker script served with a non-JS type.
#[tokio::test]
async fn the_built_pdf_worker_is_served_as_javascript() {
    let dist =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/hq-web/dist/client");
    let Some(worker) = std::fs::read_dir(dist.join("assets")).ok().and_then(|d| {
        d.filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.starts_with("pdf.worker"))
    }) else {
        built_app_missing("pdf.worker asset");
        return;
    };
    let vault = tempfile::TempDir::new().unwrap();
    let state = WsState::new(vault.path().to_path_buf(), Some(dist));
    let app = create_router(Arc::new(state));
    let res = get(&app, &format!("/assets/{worker}")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let ctype = res.headers()["content-type"].to_str().unwrap();
    assert!(ctype.contains("javascript"), "{ctype}");
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn active_vault_files_download_in_a_sandbox() {
    let (app, _v, _w) = app_with_vault(&[
        ("pic.svg", "<svg onload=alert(1)/>"),
        ("page.html", "<script>alert(1)</script>"),
        ("data.xml", "<a/>"),
        ("UPPER.SVG", "<svg/>"),
        ("photo.png", "x"),
        ("note.txt", "hi"),
    ]);
    for name in ["pic.svg", "page.html", "data.xml", "UPPER.SVG"] {
        let res = get(&app, &format!("/api/vault-asset?path={name}")).await;
        assert_eq!(res.status(), StatusCode::OK, "{name}");
        assert_eq!(res.headers()["content-disposition"], "attachment", "{name}");
        assert_eq!(
            res.headers()["content-security-policy"],
            "sandbox",
            "{name}"
        );
        assert_eq!(res.headers()["x-content-type-options"], "nosniff", "{name}");
    }
    for name in ["photo.png", "note.txt"] {
        let res = get(&app, &format!("/api/vault-asset?path={name}")).await;
        assert!(!res.headers().contains_key("content-disposition"), "{name}");
        assert_ne!(
            res.headers()["content-security-policy"],
            "sandbox",
            "{name}"
        );
    }
}

#[tokio::test]
async fn office_files_download_with_their_own_type_and_a_safe_name() {
    let (app, _v, _w) = app_with_vault(&[("AI stocks.xlsx", "x"), ("deck.pptx", "x")]);
    let res = get(&app, "/api/vault-asset?path=AI%20stocks.xlsx").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()["content-type"],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        res.headers()["content-disposition"],
        "attachment; filename=\"AI stocks.xlsx\""
    );
    let deck = get(&app, "/api/vault-asset?path=deck.pptx").await;
    assert!(
        deck.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
}

#[cfg(feature = "suite")]
#[tokio::test]
async fn the_preview_route_reads_a_note_and_refuses_private_folders_and_missing_files() {
    let (app, _v, _w) =
        app_with_vault(&[("n.md", "---\ntitle: \"Plan\"\n---\n\n# Plan\n\nhello\n")]);
    let ok = get(&app, "/api/suite/preview?path=n.md").await;
    assert_eq!(ok.status(), StatusCode::OK);
    let body = axum::body::to_bytes(ok.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["title"], "Plan");
    assert!(json["markdown"].as_str().unwrap().contains("hello"));
    for bad in ["_system/x.md", "missing.xlsx", "../n.md"] {
        let res = get(&app, &format!("/api/suite/preview?path={bad}")).await;
        assert!(res.status().is_client_error(), "{bad}: {}", res.status());
    }
}

#[cfg(feature = "suite")]
#[tokio::test]
async fn the_export_routes_list_a_notes_formats_and_make_the_file_in_exports() {
    let sheet = "---\ntitle: \"Stock\"\nsuite_kind: sheet\nsuite_fidelity: native\n---\n\n# Stock\n\n```sheet name=Stock types=\"text,int\"\n| item | qty |\n|------|-----|\n| Sugar | 4 |\n```\n";
    let (app, vault, _w) = app_with_vault(&[("stock.md", sheet)]);
    let listed = get(&app, "/api/suite/export-formats?path=stock.md").await;
    let body = axum::body::to_bytes(listed.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["formats"], serde_json::json!(["xlsx", "csv", "pdf"]));

    let post = |format: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/suite/export")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"path":"stock.md","format":"{format}"}}"#
            )))
            .unwrap()
    };
    let ok = app.clone().oneshot(post("xlsx")).await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let body = axum::body::to_bytes(ok.into_body(), usize::MAX)
        .await
        .unwrap();
    let made: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rel = made["files"][0]["path"].as_str().unwrap();
    assert!(
        rel.starts_with("Exports/") && vault.path().join(rel).exists(),
        "{made}"
    );
    assert!(made["fidelity"].as_str().unwrap().contains("kept"));
    // The note page's own Export gives the same workbook and says what it kept.
    let direct = get(&app, "/api/note/export?path=stock.md&format=xlsx").await;
    assert_eq!(direct.status(), StatusCode::OK);
    assert!(
        direct.headers()["x-export-note"]
            .to_str()
            .unwrap()
            .contains("Formulas")
            || direct.headers()["x-export-note"]
                .to_str()
                .unwrap()
                .contains("Values")
    );
    assert!(
        direct.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("stock.xlsx")
    );
    let wrong = app.clone().oneshot(post("pptx")).await.unwrap();
    assert!(wrong.status().is_client_error());
}

#[tokio::test]
async fn vault_status_says_how_many_notes_are_indexed() {
    let (app, _v, _w) = app_with_vault(&[]);
    let res = get(&app, "/api/vault-status").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["vault_path"].is_string());
    assert_eq!(json["notes"], 0, "an empty vault has no indexed notes");
}
