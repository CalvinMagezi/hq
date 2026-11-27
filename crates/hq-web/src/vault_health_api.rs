//! The notes health report for the Notes screen on the web and the phone.
//!
//! `GET /api/vault/health` is the last scan with what it could fix; `POST /api/vault/health/scan`
//! scans now; `POST /api/vault/health/ignore` and `/unignore` leave an item out of the report or
//! count it again; `/set-aside` leaves everything that is here now alone, except what can be
//! repaired, and `/count-again` counts it all again; `POST /api/vault/health/apply` does fixes and `/undo` takes a run back.
//! Behind the same owner auth as every `/api` route (an MCP key never reaches it), and served
//! under Lite (the `/api/vault` prefix). Only `apply` and `undo` change notes, and only notes
//! the planner says may be changed; each run saves the originals first. The report names notes
//! and the links written in them and stays on this device.

use axum::{Json, extract::State, response::IntoResponse};
use hq_vault::health::kind;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

/// The place pages subscribe to (`vault-health:change`) to refetch after a scan or an ignore.
const PLACE: &str = "vault-health";

const KINDS: &[&str] = &[
    kind::DEAD_LINK,
    kind::AMBIGUOUS_LINK,
    kind::BROKEN_ANCHOR,
    kind::LOOSE,
    kind::EMPTY,
    kind::DUPLICATE,
    kind::FRONTMATTER,
    kind::SIMILAR,
];

/// The longest item key accepted: a note path and a link, with room to spare.
const MAX_KEY: usize = 2000;

/// How many past scans the trend carries.
const TREND_POINTS: i64 = 30;

/// How many past repairs the screens list.
const REPAIRS_LISTED: i64 = 10;

/// What the screens read: whether a scan has run, the report, what it could fix, what was
/// fixed lately, and the trend.
async fn view(state: &Arc<WsState>) -> Result<Value, ApiError> {
    let db = state.db.clone();
    let (latest, trend, repairs) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        Ok((
            hq_tools::vault_health::latest(&db)?,
            hq_tools::vault_health::trend(&db, TREND_POINTS)?,
            db.with_conn(|c| hq_db::vault_health::repairs(c, REPAIRS_LISTED))?,
        ))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    Ok(json!({
        "checked": latest.is_some(),
        "checked_at": latest.as_ref().map(|s| s.ran_at.as_str()),
        "report": latest.as_ref().map(|s| &s.report),
        "fixes": latest.as_ref().map(|s| &s.fixes),
        "fix_counts": latest.as_ref().map(|s| json!({ "a": s.fix_a, "b": s.fix_b })),
        "skipped": latest.as_ref().map(|s| &s.skipped),
        "repairs": repairs,
        "trend": trend,
    }))
}

pub(crate) async fn health_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(view(&state).await?))
}

/// Scan now. A scan reads every note, so it stays off the runtime threads.
async fn rescan(state: &Arc<WsState>) -> Result<(), ApiError> {
    let (vault, db) = (state.vault_path.clone(), state.db.clone());
    tokio::task::spawn_blocking(move || hq_tools::vault_health::scan_and_store(&vault, &db))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    state.announce_change(PLACE);
    Ok(())
}

pub(crate) async fn scan_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    rescan(&state).await?;
    Ok(Json(view(&state).await?))
}

#[derive(Deserialize)]
pub(crate) struct IgnoreBody {
    kind: String,
    key: String,
}

fn check(body: &IgnoreBody) -> Result<(), ApiError> {
    if !KINDS.contains(&body.kind.as_str()) {
        return Err(ApiError::bad_request("unknown kind of item"));
    }
    if body.key.is_empty() || body.key.len() > MAX_KEY {
        return Err(ApiError::bad_request("the item is not named"));
    }
    Ok(())
}

pub(crate) async fn ignore_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<IgnoreBody>,
) -> Result<impl IntoResponse, ApiError> {
    check(&body)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || hq_tools::vault_health::ignore(&db, &body.kind, &body.key))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    // The counts change at once, not at the next scheduled scan.
    rescan(&state).await?;
    Ok(Json(view(&state).await?))
}

pub(crate) async fn unignore_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<IgnoreBody>,
) -> Result<impl IntoResponse, ApiError> {
    check(&body)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        hq_tools::vault_health::unignore(&db, &body.kind, &body.key)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    rescan(&state).await?;
    Ok(Json(view(&state).await?))
}

/// Leave everything that is here now alone, except what can be repaired, so the card counts only
/// what is new. Changes no note. Answers the report and how many items were set aside.
pub(crate) async fn set_aside_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault, db) = (state.vault_path.clone(), state.db.clone());
    let set =
        tokio::task::spawn_blocking(move || hq_tools::vault_health::set_rest_aside(&vault, &db))
            .await
            .map_err(ApiError::internal)?
            .map_err(ApiError::internal)?;
    rescan(&state).await?;
    let mut v = view(&state).await?;
    v["set_aside_now"] = json!(set);
    Ok(Json(v))
}

/// Count everything set aside or ignored again.
pub(crate) async fn count_again_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || hq_tools::vault_health::count_again(&db))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    rescan(&state).await?;
    Ok(Json(view(&state).await?))
}

/// Tells open pages that these notes changed on disk, so the one showing a note reloads it.
fn announce_rewritten<'a>(state: &Arc<WsState>, notes: impl Iterator<Item = &'a String>) {
    for rel in notes {
        if let Some(hash) = crate::vault_api::file_hash(&state.vault_path, rel) {
            state.announce_note(rel, &hash);
        }
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct ApplyBody {
    /// Do these fixes, by the ids the last scan listed.
    #[serde(default)]
    ids: Vec<String>,
    /// Without ids: `a` (the default) for the certain fixes, `all` to add the likely ones.
    #[serde(default)]
    tier: Option<String>,
}

pub(crate) async fn apply_handler(
    State(state): State<Arc<WsState>>,
    body: Option<Json<ApplyBody>>,
) -> Result<impl IntoResponse, ApiError> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let which = if !body.ids.is_empty() {
        hq_tools::vault_repair::Which::Ids(body.ids.into_iter().collect())
    } else {
        match body.tier.as_deref().unwrap_or("a") {
            "a" => hq_tools::vault_repair::Which::TierA,
            "all" => hq_tools::vault_repair::Which::All,
            _ => return Err(ApiError::bad_request("tier is a or all")),
        }
    };
    let (vault, db) = (state.vault_path.clone(), state.db.clone());
    let applied = tokio::task::spawn_blocking(move || {
        hq_tools::vault_repair::apply_repairs(
            &vault,
            &std::sync::Arc::new(db),
            &which,
            hq_db::vault_health::Source::Owner,
        )
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    // The notes changed, so the counts did too; and a note open on another screen reloads.
    rescan(&state).await?;
    announce_rewritten(&state, applied.done.rewritten.iter());
    Ok(Json(
        json!({ "applied": applied, "health": view(&state).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct UndoBody {
    run_id: String,
}

pub(crate) async fn undo_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<UndoBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (vault, db) = (state.vault_path.clone(), state.db.clone());
    let undone = tokio::task::spawn_blocking(move || {
        hq_tools::vault_repair::undo_repair(&vault, &std::sync::Arc::new(db), &body.run_id)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|e| ApiError::bad_request(e.to_string()))?;
    rescan(&state).await?;
    announce_rewritten(
        &state,
        undone.restored.iter().chain(undone.untrashed.iter()),
    );
    Ok(Json(
        json!({ "undone": undone, "health": view(&state).await? }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn state_with_a_broken_link() -> (tempfile::TempDir, Arc<WsState>) {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(
            vault.path().join("Notebooks/A.md"),
            "see [[Gone]] in a note with enough words to be a real note",
        )
        .unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        (vault, state)
    }

    async fn json_of(r: impl IntoResponse) -> Value {
        let body = to_bytes(r.into_response().into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn setting_the_rest_aside_hides_the_old_mess_and_counting_again_brings_it_back() {
        let (_v, state) = state_with_a_broken_link();
        let v = json_of(
            set_aside_handler(State(state.clone()))
                .await
                .map_err(|_| "failed")
                .unwrap(),
        )
        .await;
        assert_eq!(v["report"]["counts"]["links"]["dead"], 0);
        assert!(v["report"]["counts"]["set_aside"].as_u64().unwrap() >= 1);
        assert!(v["set_aside_now"].as_u64().unwrap() >= 1);
        let v = json_of(
            count_again_handler(State(state))
                .await
                .map_err(|_| "failed")
                .unwrap(),
        )
        .await;
        assert_eq!(v["report"]["counts"]["links"]["dead"], 1);
        assert_eq!(v["report"]["counts"]["set_aside"], 0);
    }

    #[tokio::test]
    async fn before_any_scan_the_report_says_it_has_not_been_checked() {
        let (_v, state) = state_with_a_broken_link();
        let v = json_of(
            health_handler(State(state))
                .await
                .map_err(|_| "failed")
                .unwrap(),
        )
        .await;
        assert_eq!(v["checked"], false);
        assert!(v["report"].is_null());
    }

    #[tokio::test]
    async fn a_scan_finds_the_problem_and_ignoring_it_clears_it_until_it_is_counted_again() {
        let (_v, state) = state_with_a_broken_link();
        let v = json_of(
            scan_handler(State(state.clone()))
                .await
                .map_err(|_| "failed")
                .unwrap(),
        )
        .await;
        assert_eq!(v["checked"], true);
        assert_eq!(v["report"]["counts"]["links"]["dead"], 1);
        let key = v["report"]["dead_links"][0]["note"]
            .as_str()
            .unwrap()
            .to_string()
            + "|[[Gone]]";

        let body = || {
            Json(IgnoreBody {
                kind: "dead_link".into(),
                key: key.clone(),
            })
        };
        let v = json_of(
            ignore_handler(State(state.clone()), body())
                .await
                .map_err(|_| "failed")
                .unwrap(),
        )
        .await;
        assert_eq!(v["report"]["counts"]["links"]["dead"], 0);
        assert_eq!(v["trend"].as_array().unwrap().len(), 2);

        let v = json_of(
            unignore_handler(State(state), body())
                .await
                .map_err(|_| "failed")
                .unwrap(),
        )
        .await;
        assert_eq!(v["report"]["counts"]["links"]["dead"], 1);
    }

    #[test]
    fn only_known_kinds_and_named_items_are_accepted() {
        let ok = |kind: &str, key: &str| {
            check(&IgnoreBody {
                kind: kind.into(),
                key: key.into(),
            })
            .is_ok()
        };
        assert!(ok("loose", "Notebooks/A.md"));
        assert!(!ok("anything", "Notebooks/A.md"));
        assert!(!ok("loose", ""));
        assert!(!ok("loose", &"x".repeat(MAX_KEY + 1)));
    }
}
