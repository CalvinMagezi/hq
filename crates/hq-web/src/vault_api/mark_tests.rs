use super::{FRONT_MATTER_CAP, MARK_HASH_CAP, has_tag, note_meta, tags_of_file, with_marks};
use crate::{WsState, create_router};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use hq_db::provenance::{Attribution, SourceKind, record};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;
use tower::ServiceExt;

const BODY_LIMIT: usize = 1 << 20;
const NOTE: &str = "---\ntags: [work, idea]\n---\nbody about pelicans";

fn setup() -> (tempfile::TempDir, Arc<WsState>, axum::Router) {
    let v = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(v.path().join("Notebooks/Inbox")).unwrap();
    std::fs::write(v.path().join("Notebooks/Inbox/a.md"), NOTE).unwrap();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let app = create_router(state.clone());
    (v, state, app)
}

fn hq_wrote(state: &WsState, rel: &str, raw: &str) {
    let hash = hq_db::search::content_hash(raw);
    state
        .db
        .with_conn(|c| {
            hq_db::search::index_note(c, rel, "a", raw, "work idea")?;
            record(
                c,
                rel,
                &hash,
                &Attribution {
                    actor_id: Some("helper"),
                    source_kind: Some(SourceKind::Agent),
                    session_id: Some("s1"),
                    task_id: None,
                },
            )
            .map(|_| ())
        })
        .unwrap();
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_note_carries_who_wrote_it_why_and_its_tags_and_new_clears_on_seen() {
    let (_v, state, app) = setup();
    let rel = "Notebooks/Inbox/a.md";
    let (_, mine) = call(&app, "GET", &format!("/api/note?path={rel}"), "").await;
    assert_eq!(mine["written_by"], "you");
    assert_eq!(mine["tags"], json!(["work", "idea"]));

    hq_wrote(&state, rel, NOTE);
    let (_, note) = call(&app, "GET", &format!("/api/note?path={rel}"), "").await;
    assert_eq!(note["written_by"], "hq");
    assert_eq!(note["review"], "new");
    assert!(
        note["why"]
            .as_str()
            .unwrap()
            .starts_with("Saved from your chat, ")
    );

    // Reading did not clear it; the explicit call does.
    let (_, again) = call(&app, "GET", &format!("/api/note?path={rel}"), "").await;
    assert_eq!(again["review"], "new");
    let body = json!({ "path": rel }).to_string();
    let (status, _) = call(&app, "POST", "/api/note/seen", &body).await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = call(&app, "GET", &format!("/api/note?path={rel}"), "").await;
    assert_eq!(after["review"], Value::Null);
    assert_eq!(after["written_by"], "hq");

    // The folder listing carries the same fields.
    let (_, listing) = call(&app, "GET", "/api/note?path=Notebooks/Inbox", "").await;
    assert_eq!(listing["entries"][0]["written_by"], "hq");
    assert_eq!(listing["entries"][0]["tags"], json!(["work", "idea"]));
}

#[tokio::test]
async fn opening_a_changed_note_leaves_it_changed() {
    let (v, state, app) = setup();
    let rel = "Notebooks/Inbox/a.md";
    let owner_hash = hq_db::search::content_hash(NOTE);
    state
        .db
        .with_conn(|c| {
            record(
                c,
                rel,
                &owner_hash,
                &Attribution {
                    actor_id: Some("owner"),
                    source_kind: Some(SourceKind::Human),
                    session_id: None,
                    task_id: None,
                },
            )
            .map(|_| ())
        })
        .unwrap();
    let edited = format!("{NOTE}\nHQ added a line");
    std::fs::write(v.path().join(rel), &edited).unwrap();
    hq_wrote(&state, rel, &edited);
    let uri = format!("/api/note?path={rel}");
    let (_, note) = call(&app, "GET", &uri, "").await;
    assert_eq!(note["review"], "changed");
    let body = json!({ "path": rel }).to_string();
    call(&app, "POST", "/api/note/seen", &body).await;
    let (_, after) = call(&app, "GET", &uri, "").await;
    assert_eq!(after["review"], "changed");
    let keep = json!({ "path": rel, "verdict": "accepted" }).to_string();
    let (status, _) = call(&app, "POST", "/api/note/review", &keep).await;
    assert_eq!(status, StatusCode::OK);
    let (_, kept) = call(&app, "GET", &uri, "").await;
    assert_eq!(kept["review"], Value::Null);
}

#[tokio::test]
async fn seen_refuses_a_path_outside_the_notes() {
    let (_v, _state, app) = setup();
    for bad in ["../x.md", "/etc/hosts", ""] {
        let body = json!({ "path": bad }).to_string();
        let (status, _) = call(&app, "POST", "/api/note/seen", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}");
    }
}

#[tokio::test]
async fn search_filters_by_who_wrote_it_and_tag_and_refuses_a_climbing_folder() {
    let (v, state, app) = setup();
    for n in 0..5 {
        let rel = format!("Notebooks/Inbox/m{n}.md");
        let raw = "pelicans pelicans pelicans";
        std::fs::write(v.path().join(&rel), raw).unwrap();
        state
            .db
            .with_conn(|c| hq_db::search::index_note(c, &rel, "m", raw, ""))
            .unwrap();
    }
    hq_wrote(&state, "Notebooks/Inbox/a.md", NOTE);
    let (_, hits) = call(
        &app,
        "GET",
        "/api/search?q=pelicans&written_by=hq&limit=1",
        "",
    )
    .await;
    assert_eq!(hits["results"][0]["note_path"], "Notebooks/Inbox/a.md");
    let (_, hits) = call(&app, "GET", "/api/search?q=pelicans&tag=idea&limit=1", "").await;
    assert_eq!(hits["results"][0]["note_path"], "Notebooks/Inbox/a.md");
    let (status, _) = call(&app, "GET", "/api/search?q=pelicans&written_by=you", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for bad in ["..%2Fx", "%2Fetc", "a%5Cb", "a/../b"] {
        let uri = format!("/api/search?q=pelicans&folder={bad}");
        let (status, _) = call(&app, "GET", &uri, "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn seen_reads_the_path_as_a_note_read_does_and_keeps_one_row() {
    let (v, state, app) = setup();
    let rel = "Notebooks/Inbox/a.md";
    hq_wrote(&state, rel, NOTE);
    for form in [
        "./Notebooks/Inbox/a.md",
        "Notebooks/Inbox/a",
        " Notebooks/Inbox/a.md ",
    ] {
        let body = json!({ "path": form }).to_string();
        let (status, _) = call(&app, "POST", "/api/note/seen", &body).await;
        assert_eq!(status, StatusCode::OK, "{form}");
    }
    let rows: Vec<String> = state
        .db
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT path FROM note_seen")?;
            Ok(stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(rows, [rel]);
    let (_, note) = call(&app, "GET", &format!("/api/note?path={rel}"), "").await;
    assert_eq!(note["review"], Value::Null);

    let missing = json!({ "path": "Notebooks/Inbox/none.md" }).to_string();
    assert_eq!(
        call(&app, "POST", "/api/note/seen", &missing).await.0,
        StatusCode::NOT_FOUND
    );
    std::fs::write(v.path().join("Notebooks/Inbox/pic.png"), "x").unwrap();
    let png = json!({ "path": "Notebooks/Inbox/pic.png" }).to_string();
    assert_eq!(
        call(&app, "POST", "/api/note/seen", &png).await.0,
        StatusCode::NOT_FOUND
    );
    let big = vec![b'a'; (MARK_HASH_CAP + 1) as usize];
    std::fs::write(v.path().join("Notebooks/Inbox/big.md"), big).unwrap();
    let body = json!({ "path": "Notebooks/Inbox/big.md" }).to_string();
    assert_eq!(
        call(&app, "POST", "/api/note/seen", &body).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[test]
fn tags_match_whole_case_folded_and_survive_a_long_front_matter() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    };
    let multi = write("m.md", "---\ntags: [\"machine learning\", Rust]\n---\nx");
    let tags = tags_of_file(&multi);
    assert!(has_tag(&tags, "machine learning"));
    assert!(has_tag(&tags, "#RUST"));
    assert!(!has_tag(&tags, "learning"));
    // Tags after more than 4096 bytes of other front matter are still read.
    let padding = "note: ".to_string() + &"x".repeat(6000);
    let long = write("l.md", &format!("---\n{padding}\ntags: [late]\n---\nx"));
    assert_eq!(tags_of_file(&long), ["late"]);
    // The list view reads them the same way.
    let meta = note_meta(dir.path(), &long).unwrap();
    assert_eq!(meta.value["tags"], json!(["late"]));
}

#[test]
fn an_alias_bomb_in_the_front_matter_is_refused_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let mut fm = String::from("a: &a [x,x,x,x,x,x,x,x,x]\n");
    for n in 0..1500 {
        let prev = if n == 0 {
            "a".to_string()
        } else {
            format!("l{}", n - 1)
        };
        fm.push_str(&format!("l{n}: &l{n} [*{prev},*{prev},*{prev},*{prev}]\n"));
    }
    assert!(
        fm.len() > 30_000 && fm.len() < FRONT_MATTER_CAP,
        "{}",
        fm.len()
    );
    fm.push_str("tags: [secret]\n");
    let path = dir.path().join("bomb.md");
    std::fs::write(&path, format!("---\n{fm}---\nbody")).unwrap();
    let started = Instant::now();
    assert!(tags_of_file(&path).is_empty());
    let meta = note_meta(dir.path(), &path);
    assert!(started.elapsed() < std::time::Duration::from_millis(200));
    assert!(meta.is_none_or(|m| m.value["tags"] == json!([])));
}

#[tokio::test]
async fn an_outside_edit_drops_a_note_from_the_hq_search_and_the_dot() {
    let (v, state, app) = setup();
    let rel = "Notebooks/Inbox/a.md";
    hq_wrote(&state, rel, NOTE);
    let (_, signals) = call(&app, "GET", "/api/vault-signals", "").await;
    assert_eq!(signals["has_new"], true);
    let (_, hits) = call(&app, "GET", "/api/search?q=pelicans&written_by=hq", "").await;
    assert_eq!(hits["results"].as_array().unwrap().len(), 1);

    std::fs::write(v.path().join(rel), format!("{NOTE} edited elsewhere")).unwrap();
    let (_, hits) = call(&app, "GET", "/api/search?q=pelicans&written_by=hq", "").await;
    assert!(hits["results"].as_array().unwrap().is_empty(), "{hits}");
    let (_, note) = call(&app, "GET", &format!("/api/note?path={rel}"), "").await;
    assert_eq!(note["written_by"], "you");
    // The vault-signals walk is cached for a few seconds; ask the function directly.
    let marked = with_marks(&state, json!({"recent": [{"path": rel}]}));
    assert_eq!(marked["has_new"], false);
    assert_eq!(marked["recent"][0]["written_by"], "you");
}

#[tokio::test]
async fn a_filtered_search_walk_finds_a_match_past_hundreds_of_other_files() {
    let (v, _state, app) = setup();
    for n in 0..650 {
        std::fs::write(
            v.path().join(format!("Notebooks/Inbox/f{n}.md")),
            "---\ntags: [other]\n---\nquokka",
        )
        .unwrap();
    }
    std::fs::write(
        v.path().join("Notebooks/Inbox/target.md"),
        "---\ntags: [Wanted]\n---\nquokka",
    )
    .unwrap();
    let (_, hits) = call(&app, "GET", "/api/search?q=quokka&tag=wanted&limit=1", "").await;
    assert_eq!(
        hits["results"][0]["note_path"], "Notebooks/Inbox/target.md",
        "{hits}"
    );
    let (_, none) = call(&app, "GET", "/api/search?q=quokka&tag=learning", "").await;
    assert!(none["results"].as_array().unwrap().is_empty());
}
