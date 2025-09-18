use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::api::AppState;
use crate::common::ChatUserId;
use crate::launch::subscription::db::SubscriptionError;
use crate::launch::subscription::{PlanType, Subscription};

/// API error type for consistent error responses
#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "message")]
pub enum ApiError {
    AlreadyExists(ChatUserId),
    PlanNotFound(PlanType),
    Internal,
}

impl From<SubscriptionError> for ApiError {
    fn from(err: SubscriptionError) -> Self {
        match err {
            SubscriptionError::AlreadyExists(msg) => ApiError::AlreadyExists(msg),
            SubscriptionError::PlanNotFound(plan) => ApiError::PlanNotFound(plan),
            SubscriptionError::Other(e) => {
                error!(?e, "Internal server error");
                ApiError::Internal
            }
            SubscriptionError::DatabaseError(error) => {
                error!(?error, "Database error");
                ApiError::Internal
            }
            SubscriptionError::DatabasePoolError(pool_error) => {
                error!(?pool_error, "Database pool error");
                ApiError::Internal
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::AlreadyExists(_) => StatusCode::CONFLICT,
            ApiError::PlanNotFound(_) => StatusCode::BAD_REQUEST,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(self)).into_response()
    }
}

/// Get all available plans
async fn get_plan_catalog(State(_state): State<AppState>) -> Result<Json<Vec<PlanType>>, ApiError> {
    let plans = crate::launch::subscription::db::get_plan_catalog();
    Ok(Json(plans))
}

#[derive(Deserialize, Debug)]
pub struct SubscribeRequest {
    pub plan: PlanType,
}

/// Subscribe a user to a plan
async fn subscribe(
    State(state): State<AppState>,
    Path(user_id): Path<ChatUserId>,
    Json(payload): Json<SubscribeRequest>,
) -> Result<Json<Subscription>, ApiError> {
    let (subscription, _subscription_payment) = state
        .subscription_db
        .subscribe(
            &state.wallet,
            &user_id,
            payload.plan,
            state.allow_internal_invoice,
        )
        .await?;
    // Don't return the subscription payment to client as it is an internal affair
    Ok(Json(subscription))
}

/// Get current subscription for a user
async fn get_current_subscription(
    State(state): State<AppState>,
    Path(user_id): Path<ChatUserId>,
) -> Result<Json<Option<Subscription>>, ApiError> {
    let sub = state
        .subscription_db
        .get_current_subscription(&user_id)
        .await?;
    Ok(Json(sub))
}

/// Sets up and runs the Axum server for subscription API
pub fn build_routes() -> Router<AppState> {
    Router::new()
        .route("/plans", get(get_plan_catalog))
        .route("/users/{user_id}/subscribe", post(subscribe))
        .route("/users/{user_id}", get(get_current_subscription))
}
