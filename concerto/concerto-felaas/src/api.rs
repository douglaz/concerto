use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use concerto_common::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type ApiResult<T> = std::result::Result<T, StatusCode>;
use tracing::info;

#[derive(Clone)]
pub struct ApiState {
    pub provider: crate::provider::FeLaaSProvider,
    pub deployment: crate::deployment::DeploymentBackend,
    pub db_pool: sqlx::PgPool,
}

pub fn create_app(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/info", get(provider_info))
        .route("/slots", get(list_slots))
        .route("/slots/:id", get(get_slot))
        .route("/slots/allocate", post(allocate_slot))
        .route("/slots/:id/release", post(release_slot))
        .route("/slots/:id/status", get(slot_status))
        .route("/economics/pricing", get(get_pricing))
        .route("/economics/utilization", get(get_utilization))
        .with_state(Arc::new(state))
}

async fn health_check() -> StatusCode {
    StatusCode::OK
}

async fn provider_info(
    State(state): State<Arc<ApiState>>,
) -> ApiResult<Json<ProviderInfoResponse>> {
    match state.provider.get_provider_info().await {
        Ok(info) => Ok(Json(ProviderInfoResponse {
            name: info.name,
            regions: info.regions,
            features: info.features,
            minimum_subscription_tier: format!("{:?}", info.minimum_subscription),
            availability: info.availability,
            reputation_score: info.reputation_score,
        })),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn list_slots(
    State(state): State<Arc<ApiState>>,
) -> ApiResult<Json<Vec<SlotInfo>>> {
    match crate::database::list_slots(&state.db_pool).await {
        Ok(slots) => {
            let slot_infos: Vec<SlotInfo> = slots
                .into_iter()
                .map(|s| SlotInfo {
                    slot_id: s.slot_id.to_string(),
                    federation_id: s.federation_id,
                    guardian_npub: s.guardian_npub,
                    status: s.status,
                    allocated_at: s.allocated_at.to_rfc3339(),
                })
                .collect();
            Ok(Json(slot_infos))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_slot(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
) -> ApiResult<Json<SlotInfo>> {
    match crate::database::get_slot(&state.db_pool, &id).await {
        Ok(slot) => Ok(Json(SlotInfo {
            slot_id: slot.slot_id.to_string(),
            federation_id: slot.federation_id,
            guardian_npub: slot.guardian_npub,
            status: slot.status,
            allocated_at: slot.allocated_at.to_rfc3339(),
        })),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn allocate_slot(
    State(state): State<Arc<ApiState>>,
    Json(request): Json<AllocateSlotRequest>,
) -> ApiResult<Json<SlotEndpoint>> {
    info!("Allocating slot for federation {}", request.federation_id);
    
    match state.provider.allocate_slot(request).await {
        Ok(endpoint) => Ok(Json(endpoint)),
        Err(e) => {
            tracing::error!("Failed to allocate slot: {}", e);
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

async fn release_slot(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
) -> ApiResult<StatusCode> {
    let slot_id = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    
    match state.provider.release_slot(slot_id).await {
        Ok(_) => Ok(StatusCode::OK),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn slot_status(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
) -> Result<Json<SlotStatusResponse>, StatusCode> {
    match crate::database::get_slot(&state.db_pool, &id).await {
        Ok(slot) => Ok(Json(SlotStatusResponse {
            slot_id: slot.slot_id.to_string(),
            status: slot.status,
            health: "healthy".to_string(),
            uptime_seconds: 0,
            resource_usage: ResourceUsageInfo {
                cpu_percent: 0.0,
                memory_used_gb: 0.0,
                storage_used_gb: 0.0,
                bandwidth_used_mbps: 0.0,
            },
        })),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_pricing(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<PricingResponse>, StatusCode> {
    match crate::database::get_current_pricing(&state.db_pool).await {
        Ok(pricing) => Ok(Json(PricingResponse {
            base_slot_price_sats: pricing.base_slot_price_sats,
            cpu_per_core_hour_sats: pricing.cpu_per_core_hour_sats,
            memory_per_gb_hour_sats: pricing.memory_per_gb_hour_sats,
            storage_per_gb_month_sats: pricing.storage_per_gb_month_sats,
            bandwidth_per_gb_sats: pricing.bandwidth_per_gb_sats,
        })),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_utilization(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<UtilizationResponse>, StatusCode> {
    match crate::database::get_utilization_report(&state.db_pool).await {
        Ok(util) => Ok(Json(UtilizationResponse {
            total_slots: util.total_slots,
            active_slots: util.active_slots,
            utilization_percent: util.utilization_percent,
            avg_cpu_percent: util.avg_cpu_percent,
            avg_memory_percent: util.avg_memory_percent,
        })),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// Response DTOs
#[derive(Serialize)]
struct ProviderInfoResponse {
    name: String,
    regions: Vec<String>,
    features: Vec<String>,
    minimum_subscription_tier: String,
    availability: f32,
    reputation_score: f32,
}

#[derive(Serialize)]
struct SlotInfo {
    slot_id: String,
    federation_id: String,
    guardian_npub: String,
    status: String,
    allocated_at: String,
}

#[derive(Serialize)]
struct SlotStatusResponse {
    slot_id: String,
    status: String,
    health: String,
    uptime_seconds: u64,
    resource_usage: ResourceUsageInfo,
}

#[derive(Serialize)]
struct ResourceUsageInfo {
    cpu_percent: f32,
    memory_used_gb: f32,
    storage_used_gb: f32,
    bandwidth_used_mbps: f32,
}

#[derive(Serialize)]
struct PricingResponse {
    base_slot_price_sats: u64,
    cpu_per_core_hour_sats: u64,
    memory_per_gb_hour_sats: u64,
    storage_per_gb_month_sats: u64,
    bandwidth_per_gb_sats: u64,
}

#[derive(Serialize)]
struct UtilizationResponse {
    total_slots: u32,
    active_slots: u32,
    utilization_percent: f32,
    avg_cpu_percent: f32,
    avg_memory_percent: f32,
}