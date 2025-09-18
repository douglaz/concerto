use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::api::AppState;
use crate::common::ChatUserId;
use crate::og_registry::db::OgRecord;
use crate::og_registry::error::OgRegistryError;

#[derive(Debug, Deserialize)]
pub struct UpsertOgRequest {
    pub reference_user_id: ChatUserId,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct OgResponse {
    pub user_id: ChatUserId,
    pub reference_user_id: ChatUserId,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<OgRecord> for OgResponse {
    fn from(record: OgRecord) -> Self {
        OgResponse {
            user_id: record.user_id,
            reference_user_id: record.reference_user_id,
            is_active: record.is_active,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ListOgsResponse {
    pub ogs: Vec<OgResponse>,
}

/// Build the OG registry routes
pub fn build_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_active_ogs))
        .route("/{user_id}", put(upsert_og).get(get_og))
}

/// Upsert an OG (create or update)
async fn upsert_og(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(payload): Json<UpsertOgRequest>,
) -> Result<Json<OgResponse>, StatusCode> {
    let user_id = ChatUserId::from(user_id);

    match state
        .og_registry_db
        .upsert_og(user_id, payload.reference_user_id, payload.is_active)
        .await
    {
        Ok(record) => Ok(Json(record.into())),
        Err(e) => match e {
            OgRegistryError::DuplicateActiveOg {
                reference_user_id,
                existing_user_id,
            } => {
                error!(
                    %reference_user_id,
                    %existing_user_id,
                    "Conflict: Another OG with same reference_user_id is already active"
                );
                Err(StatusCode::CONFLICT)
            }
            _ => {
                error!(?e, "Failed to upsert OG");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
    }
}

/// Get a specific OG
async fn get_og(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<OgResponse>, StatusCode> {
    let user_id = ChatUserId::from(user_id);

    match state.og_registry_db.get_og(&user_id).await {
        Ok(Some(record)) => Ok(Json(record.into())),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            error!(?e, "Failed to get OG");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// List all active OGs
async fn list_active_ogs(
    State(state): State<AppState>,
) -> Result<Json<ListOgsResponse>, StatusCode> {
    match state.og_registry_db.list_active_ogs().await {
        Ok(records) => {
            let ogs = records.into_iter().map(OgResponse::from).collect();
            Ok(Json(ListOgsResponse { ogs }))
        }
        Err(e) => {
            error!(?e, "Failed to list active OGs");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
