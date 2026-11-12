//! `GET /api/capabilities`: what HQ can do on this install, in the person's words, for the empty
//! chat on the web and phone apps. Resolved from the embedded catalog against the current config,
//! the registered tools and what the owner's own sessions have used
//! (`hq_tools::capabilities`). Reads only; no model.

use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use hq_core::config::HqConfig;
use hq_tools::capabilities::{
    Install, Offer, Readiness, catalog, resolve, suggestions, used_tools,
};
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;

/// Chips on an empty chat.
const SUGGESTIONS: usize = 3;

/// `hidden` is the types the person hid or turned off (`hq_db::offers::muted`): their pills stay
/// out of the suggestions but the capability itself is still listed.
fn view(offers: &[Offer], install: &Install, hidden: &HashSet<String>) -> Value {
    let open: Vec<Offer> = offers
        .iter()
        .filter(|o| !hidden.contains(&o.id))
        .cloned()
        .collect();
    let shown: Vec<&Offer> = offers
        .iter()
        .filter(|o| o.readiness != Readiness::Unavailable)
        .collect();
    json!({
        "suggestions": suggestions(&open, SUGGESTIONS),
        "capabilities": shown,
        "needs_model": !install.model,
    })
}

pub(crate) async fn capabilities_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    // Read fresh: a key saved in setup a minute ago must turn "one step away" into "ready".
    let config = tokio::task::spawn_blocking(HqConfig::load)
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("could not read the HQ config: {e}")))?;
    let db = state.db.clone();
    let registry = state.registry.clone();
    let body = tokio::task::spawn_blocking(move || {
        let install = Install::detect(&config);
        let has_tool = |name: &str| registry.as_ref().is_none_or(|r| r.get(name).is_some());
        let offers = resolve(catalog(), &install, &has_tool, &used_tools(&db));
        let now = chrono::Utc::now().timestamp();
        let hidden = db
            .with_conn(|c| hq_db::offers::muted(c, now))
            .unwrap_or_default();
        view(&offers, &install, &hidden)
    })
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(model: bool) -> Install {
        Install {
            model,
            ocr: false,
            email: false,
            helpers: true,
        }
    }

    #[test]
    fn a_fresh_install_without_a_key_shows_the_step_and_no_chips() {
        let i = install(false);
        let offers = resolve(catalog(), &i, &|_: &str| true, &HashSet::new());
        let v = view(&offers, &i, &HashSet::new());
        assert_eq!(v["needs_model"], true);
        assert_eq!(v["suggestions"].as_array().unwrap().len(), 0);
        let shown = v["capabilities"].as_array().unwrap();
        assert!(!shown.is_empty());
        assert!(shown.iter().all(|c| c["state"] == "one_step"));
        assert!(
            shown.iter().all(|c| c["id"] != "read-photo"),
            "no OCR here, and that is not a step the app can take"
        );
    }

    #[test]
    fn with_a_key_the_chips_are_ready_untried_things() {
        let i = install(true);
        let used: HashSet<String> = ["vault_write_note".to_string()].into();
        let offers = resolve(catalog(), &i, &|_: &str| true, &used);
        let v = view(&offers, &i, &HashSet::new());
        let chips = v["suggestions"].as_array().unwrap();
        assert_eq!(chips.len(), SUGGESTIONS);
        for c in chips {
            assert_eq!(c["state"], "ready");
            assert_eq!(c["used"], false);
            assert!(c["example"].as_str().is_some_and(|e| !e.is_empty()));
        }
        assert_eq!(v["needs_model"], false);
    }

    #[test]
    fn a_hidden_capability_has_no_pill_and_the_next_one_takes_its_place() {
        let i = install(true);
        let offers = resolve(catalog(), &i, &|_: &str| true, &HashSet::new());
        let first = view(&offers, &i, &HashSet::new());
        let ids = |v: &Value| -> Vec<String> {
            v["suggestions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["id"].as_str().unwrap().to_string())
                .collect()
        };
        let before = ids(&first);
        assert_eq!(before.len(), SUGGESTIONS);
        let hidden: HashSet<String> = [before[0].clone()].into();
        let after = ids(&view(&offers, &i, &hidden));
        assert_eq!(after.len(), SUGGESTIONS);
        assert!(!after.contains(&before[0]));
        let all = view(&offers, &i, &hidden)["capabilities"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(
            all,
            first["capabilities"].as_array().unwrap().len(),
            "still listed"
        );
    }

    #[test]
    fn three_not_nows_on_a_kind_hide_its_pill() {
        let db = hq_db::Database::open_memory().unwrap();
        let hidden = db
            .with_conn(|c| {
                for (i, id) in ["a", "b", "c"].iter().enumerate() {
                    hq_db::offers::record_shown(c, id, "dated_todo", 100 + i as i64)?;
                    hq_db::offers::answer(c, id, hq_db::offers::Answer::NotNow, 200)?;
                }
                hq_db::offers::muted(c, 300)
            })
            .unwrap();
        let i = install(true);
        let offers = resolve(catalog(), &i, &|_: &str| true, &HashSet::new());
        let v = view(&offers, &i, &hidden);
        let ids: Vec<&str> = v["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert!(!ids.contains(&"track-todo"), "{ids:?}");
        assert!(ids.len() == SUGGESTIONS);
    }
}
