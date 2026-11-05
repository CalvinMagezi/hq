//! `GET /api/capabilities`: what HQ can do on this install, in the person's words, for the empty
//! chat on the web and phone apps. Resolved from the embedded catalog against the current config,
//! the registered tools and what the owner's own sessions have used
//! (`hq_tools::capabilities`). Reads only; no model.

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
const SUGGESTIONS: usize = 4;

fn view(offers: &[Offer], install: &Install) -> Value {
    let shown: Vec<&Offer> = offers
        .iter()
        .filter(|o| o.readiness != Readiness::Unavailable)
        .collect();
    json!({
        "suggestions": suggestions(offers, SUGGESTIONS),
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
        view(&offers, &install)
    })
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
        let v = view(&offers, &i);
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
        let v = view(&offers, &i);
        let chips = v["suggestions"].as_array().unwrap();
        assert_eq!(chips.len(), SUGGESTIONS);
        for c in chips {
            assert_eq!(c["state"], "ready");
            assert_eq!(c["used"], false);
            assert!(c["example"].as_str().is_some_and(|e| !e.is_empty()));
        }
        assert_eq!(v["needs_model"], false);
    }
}
