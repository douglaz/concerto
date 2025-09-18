use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use axum_extra::extract::Query;
use fedimint_core::invite_code::InviteCode;
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::api::AppState;
use crate::common::{ChatUserId, Endpoint};
use crate::launch::configuration::{
    FederationLaunchConfiguration, FederationLaunchError, FederationLaunchId,
    FederationLaunchLiquidity, FederationLaunchLiquidityStatus, FederationLaunchStatus,
};

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "message")]
pub enum ApiError {
    InvalidNumOfGuardians(String),
    InvalidNumOfFedimints(String),
    LaunchIdNotFoundOrIsInInvalidStatus(String),
    InvalidStatusTransition(String),
    NoActiveSubscription,
    PlanLimitExceededFedimints(String),
    PlanLimitExceededOgs(String),
    Internal,
}

impl From<FederationLaunchError> for ApiError {
    fn from(err: FederationLaunchError) -> Self {
        match err {
            FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus => {
                ApiError::LaunchIdNotFoundOrIsInInvalidStatus(err.to_string())
            }
            FederationLaunchError::InvalidStatusTransition(_, _) => {
                ApiError::InvalidStatusTransition(err.to_string())
            }
            FederationLaunchError::InvalidNumOfGuardians(_, _) => {
                ApiError::InvalidNumOfGuardians(err.to_string())
            }
            FederationLaunchError::InvalidNumOfFedimints(_) => {
                ApiError::InvalidNumOfFedimints(err.to_string())
            }
            FederationLaunchError::NoActiveSubscription => ApiError::NoActiveSubscription,
            FederationLaunchError::PlanLimitExceededFedimints(_, _) => {
                ApiError::PlanLimitExceededFedimints(err.to_string())
            }
            FederationLaunchError::PlanLimitExceededOgs(_, _) => {
                ApiError::PlanLimitExceededOgs(err.to_string())
            }
            FederationLaunchError::Other(error) => {
                error!(?error, "Internal server error");
                ApiError::Internal
            }
            FederationLaunchError::DatabaseError(error) => {
                error!(?error, "Database error");
                ApiError::Internal
            }
            FederationLaunchError::DatabasePoolError(pool_error) => {
                error!(?pool_error, "Database pool error");
                ApiError::Internal
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::InvalidNumOfGuardians(_)
            | ApiError::InvalidNumOfFedimints(_)
            | ApiError::InvalidStatusTransition(_)
            | ApiError::LaunchIdNotFoundOrIsInInvalidStatus(_) => StatusCode::BAD_REQUEST,
            ApiError::NoActiveSubscription => StatusCode::PAYMENT_REQUIRED,
            ApiError::PlanLimitExceededFedimints(_) | ApiError::PlanLimitExceededOgs(_) => {
                StatusCode::FORBIDDEN
            }
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(self);
        (status, body).into_response()
    }
}

#[derive(Deserialize)]
pub struct CreateFederationRequest {
    name: String,
    num_fedimints: u8,
    num_ogs: u8,
    num_guardians: u8,
    fedimint_user_ids: Vec<ChatUserId>,
    og_user_ids: Vec<ChatUserId>,
}

#[derive(Serialize)]
pub struct CreateFederationResponse {
    pub federation_launch_configuration: FederationLaunchConfiguration,
}

#[derive(Deserialize)]
pub struct SearchLaunchConfigurationsRequest {
    #[serde(default)]
    pub og_user_ids: Vec<ChatUserId>,
    #[serde(default)]
    pub fedimint_user_ids: Vec<ChatUserId>,
    #[serde(default)]
    pub status: Vec<FederationLaunchStatus>,
    #[serde(default)]
    pub liquidity_status: Option<FederationLaunchLiquidityStatus>,
}

#[derive(Serialize)]
pub struct SearchLaunchConfigurationsResponse {
    pub launch_configurations: Vec<FederationLaunchConfiguration>,
}

/// Create a new standard (bitcoin-based) federation launch
async fn create_standard_federation(
    State(state): State<AppState>,
    Path(user_id): Path<ChatUserId>,
    Json(payload): Json<CreateFederationRequest>,
) -> Result<Json<CreateFederationResponse>, ApiError> {
    let federation_launch_configuration = state
        .launcher_db
        .create_standard_federation(
            user_id,
            payload.name,
            payload.num_fedimints,
            payload.num_ogs,
            payload.num_guardians,
            payload.fedimint_user_ids,
            payload.og_user_ids,
        )
        .await?;
    Ok(Json(CreateFederationResponse {
        federation_launch_configuration,
    }))
}

/// Create a test fiat federation (staging mode only)
async fn create_test_fiat_federation(
    State(state): State<AppState>,
    Json(payload): Json<CreateFederationRequest>,
) -> Result<Json<CreateFederationResponse>, StatusCode> {
    // Return 404 if not in staging mode
    if !state.staging {
        return Err(StatusCode::NOT_FOUND);
    }

    // Use a test user ID
    let user_id = ChatUserId::from("test-user");

    let federation_launch_configuration = state
        .launcher_db
        .create_test_fiat_federation(
            user_id,
            payload.name,
            payload.num_fedimints,
            payload.num_ogs,
            payload.num_guardians,
            payload.fedimint_user_ids,
            payload.og_user_ids,
        )
        .await
        .map_err(|e| {
            error!(?e, "Failed to create test federation");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(CreateFederationResponse {
        federation_launch_configuration,
    }))
}

/// Create a new fiat federation launch
async fn create_fiat_federation(
    State(state): State<AppState>,
    Path(user_id): Path<ChatUserId>,
    Json(payload): Json<CreateFederationRequest>,
) -> Result<Json<CreateFederationResponse>, ApiError> {
    let federation_launch_configuration = state
        .launcher_db
        .create_fiat_federation(
            user_id,
            payload.name,
            payload.num_fedimints,
            payload.num_ogs,
            payload.num_guardians,
            payload.fedimint_user_ids,
            payload.og_user_ids,
        )
        .await?;
    Ok(Json(CreateFederationResponse {
        federation_launch_configuration,
    }))
}

async fn search_launch_configurations(
    State(state): State<AppState>,
    Query(payload): Query<SearchLaunchConfigurationsRequest>,
) -> Result<Json<SearchLaunchConfigurationsResponse>, ApiError> {
    let launch_configurations: Vec<FederationLaunchConfiguration> = state
        .launcher_db
        .search_launch_configurations(
            &payload.fedimint_user_ids,
            &payload.og_user_ids,
            &payload.status,
            payload.liquidity_status.as_ref(),
        )
        .await?;
    Ok(Json(SearchLaunchConfigurationsResponse {
        launch_configurations,
    }))
}

/// Get federation launch configuration
async fn get_federation_configuration(
    State(state): State<AppState>,
    Path(launch_id): Path<FederationLaunchId>,
) -> Result<Json<Option<FederationLaunchConfiguration>>, ApiError> {
    let config = state
        .launcher_db
        .get_federation_launch_configuration(&launch_id)
        .await?;
    Ok(Json(config))
}

#[derive(Deserialize)]
struct SetOgsRequest {
    pub og_user_ids: Vec<ChatUserId>,
}

/// Update OG user IDs
async fn set_ogs(
    State(state): State<AppState>,
    Path(launch_id): Path<FederationLaunchId>,
    Json(payload): Json<SetOgsRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .launcher_db
        .set_ogs(launch_id, payload.og_user_ids)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SetGuardianEndpointsRequest {
    pub guardian_endpoints: Vec<Endpoint>,
    pub admin_ui_endpoints: Vec<Endpoint>,
}

/// Update guardian endpoints
async fn set_guardian_endpoints(
    State(state): State<AppState>,
    Path(launch_id): Path<FederationLaunchId>,
    Json(payload): Json<SetGuardianEndpointsRequest>,
) -> Result<(), ApiError> {
    state
        .launcher_db
        .set_guardian_endpoint(
            launch_id,
            payload.guardian_endpoints,
            payload.admin_ui_endpoints,
        )
        .await?;
    Ok(())
}

#[derive(Deserialize)]
struct UpdateStatusRequest {
    pub new_status: FederationLaunchStatus,
}

/// Update federation launch status
async fn update_status(
    State(state): State<AppState>,
    Path(launch_id): Path<FederationLaunchId>,
    Json(payload): Json<UpdateStatusRequest>,
) -> Result<(), ApiError> {
    state
        .launcher_db
        .update_status(launch_id, payload.new_status)
        .await?;
    Ok(())
}

#[derive(Deserialize)]
struct OverrideStatusRequest {
    pub old_status: FederationLaunchStatus,
    pub new_status: FederationLaunchStatus,
}

#[derive(Deserialize)]
struct SetInviteCodeRequest {
    pub invite_code: InviteCode,
}

async fn override_status(
    State(state): State<AppState>,
    Path(launch_id): Path<FederationLaunchId>,
    Json(payload): Json<OverrideStatusRequest>,
) -> Result<(), ApiError> {
    state
        .launcher_db
        .override_status(launch_id, payload.old_status, payload.new_status)
        .await?;
    Ok(())
}

/// Update federation liquidity configuration
async fn update_liquidity(
    State(state): State<AppState>,
    Path(launch_id): Path<FederationLaunchId>,
    Json(liquidity): Json<FederationLaunchLiquidity>,
) -> Result<(), ApiError> {
    state
        .launcher_db
        .update_liquidity(launch_id, liquidity)
        .await?;
    Ok(())
}

/// Set federation invite code
async fn set_invite_code(
    State(state): State<AppState>,
    Path(launch_id): Path<FederationLaunchId>,
    Json(payload): Json<SetInviteCodeRequest>,
) -> Result<(), ApiError> {
    state
        .launcher_db
        .set_invite_code(launch_id, payload.invite_code)
        .await?;
    Ok(())
}

pub fn build_routes() -> Router<AppState> {
    let mut router = Router::new()
        .route(
            "/users/{user_id}/create-standard-federation",
            post(create_standard_federation),
        )
        .route(
            "/users/{user_id}/create-fiat-federation",
            post(create_fiat_federation),
        )
        .route(
            "/search-launch-configurations",
            get(search_launch_configurations),
        )
        .route(
            "/federations/{launch_id}",
            get(get_federation_configuration),
        )
        .route("/federations/{launch_id}/ogs", put(set_ogs))
        .route(
            "/federations/{launch_id}/guardian-endpoints",
            put(set_guardian_endpoints),
        )
        .route("/federations/{launch_id}/status", put(update_status))
        .route("/federations/{launch_id}/liquidity", put(update_liquidity))
        .route("/federations/{launch_id}/invite-code", put(set_invite_code))
        .route(
            "/federations/{launch_id}/override-status",
            put(override_status),
        );

    // Add test endpoint - it will return 404 if accessed when staging is false
    router = router.route(
        "/test/create-fiat-federation",
        post(create_test_fiat_federation),
    );

    router
}
