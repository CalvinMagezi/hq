//! The sites HQ may fetch in any chat without asking, for the owner to review, add and remove.
//! These routes sit behind the owner's web token; no tool a model can call reaches them.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use hq_db::fetch_sites;
use serde::Deserialize;
use serde_json::json;

use crate::WsState;
use crate::error::ApiError;

pub(crate) async fn list_handler(State(state): State<Arc<WsState>>) -> Response {
    let listed = state
        .db
        .with_conn(|c| Ok((fetch_sites::trusted(c)?, fetch_sites::trusted_domains(c)?)));
    match listed {
        Ok((sites, domains)) => Json(json!({ "sites": sites, "domains": domains })).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct AddDomain {
    domain: String,
}

pub(crate) async fn add_domain_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<AddDomain>,
) -> Response {
    match state
        .db
        .with_conn(|c| fetch_sites::add_trusted_domain(c, &body.domain))
    {
        Ok(domain) => Json(json!({ "ok": true, "domain": domain })).into_response(),
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}

pub(crate) async fn forget_handler(
    State(state): State<Arc<WsState>>,
    Path(host): Path<String>,
) -> Response {
    match state
        .db_blocking(move |c| fetch_sites::forget(c, &host))
        .await
    {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}
