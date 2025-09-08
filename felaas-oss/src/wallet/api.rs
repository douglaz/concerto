use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fedimint_client::OperationId;
use fedimint_ln_common::lightning_invoice::Bolt11Invoice;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

use crate::amount::Amount;
use crate::api::AppState;
use crate::common::ChatUserId;
use crate::wallet::core::{
    BalanceDetails, FelaasWalletError, InvoiceCreatedDetails, InvoicePaidDetails,
};

#[derive(Deserialize, Debug)]
pub struct CreateInvoiceRequest {
    pub amount: Amount,
    pub expire_time: Option<u64>,
    pub desc: String,
}

#[derive(Deserialize, Debug)]
pub struct PayInvoiceRequest {
    pub invoice: Bolt11Invoice,
}

#[derive(Deserialize)]
pub struct AwaitInvoiceQuery;

pub async fn create_invoice(
    State(state): State<AppState>,
    Path(user_id): Path<ChatUserId>,
    Json(payload): Json<CreateInvoiceRequest>,
) -> Result<Json<InvoiceCreatedDetails>, ApiError> {
    debug!(?payload, ?user_id, "create invoice requested");
    let desc = fedimint_ln_common::lightning_invoice::Description::new(payload.desc)
        .map_err(|e| ApiError::InvalidInvoiceDescription(e.to_string()))?;
    let details = state
        .wallet
        .create_invoice(
            &user_id,
            payload.amount,
            payload.expire_time,
            desc,
            state.allow_internal_invoice,
        )
        .await?;
    Ok(Json(details))
}

pub async fn pay_invoice(
    State(state): State<AppState>,
    Path(user_id): Path<ChatUserId>,
    Json(payload): Json<PayInvoiceRequest>,
) -> Result<Json<InvoicePaidDetails>, ApiError> {
    debug!(?payload, ?user_id, "pay invoice requested");
    let details = state.wallet.pay_invoice(&user_id, &payload.invoice).await?;
    Ok(Json(details))
}

pub async fn await_invoice(
    State(state): State<AppState>,
    Path((user_id, operation_id)): Path<(ChatUserId, OperationId)>,
) -> Result<(), ApiError> {
    state.wallet.await_invoice(&user_id, operation_id).await?;
    Ok(())
}

pub async fn get_balance(
    State(state): State<AppState>,
    Path(user_id): Path<ChatUserId>,
) -> Result<Json<BalanceDetails>, ApiError> {
    debug!(?user_id, "get balance requested");
    let balance = state.wallet.get_balance(&user_id).await?;
    Ok(Json(balance))
}

pub fn build_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/users/{user_id}/invoices",
            post(crate::wallet::api::create_invoice),
        )
        .route(
            "/users/{user_id}/payments",
            post(crate::wallet::api::pay_invoice),
        )
        .route(
            "/users/{user_id}/invoices/{operation_id}/await",
            get(crate::wallet::api::await_invoice),
        )
        .route(
            "/users/{user_id}/balance",
            get(crate::wallet::api::get_balance),
        )
}

/// API error type that wraps business and technical errors.
/// Technical errors are always mapped to a generic message for security.
#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "message")]
pub enum ApiError {
    InvoiceAmountEqualZero,
    NoInvoiceAmount,
    InvoiceCanceled,
    UnknownOperationId,
    InsufficientBalance,
    InvalidInvoiceDescription(String),
    Internal,
}

impl From<FelaasWalletError> for ApiError {
    fn from(e: FelaasWalletError) -> Self {
        match e {
            FelaasWalletError::InvoiceAmountEqualZero => ApiError::InvoiceAmountEqualZero,
            FelaasWalletError::NoInvoiceAmount => ApiError::NoInvoiceAmount,
            FelaasWalletError::InvoiceCanceled => ApiError::InvoiceCanceled,
            FelaasWalletError::UnknownOperationId => ApiError::UnknownOperationId,
            FelaasWalletError::InsufficientBalance(details) => {
                debug!(?details, "Insufficient balance");
                ApiError::InsufficientBalance
            }
            FelaasWalletError::Other(e) => {
                error!(?e, "Internal server error");
                ApiError::Internal
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::InvoiceAmountEqualZero
            | ApiError::NoInvoiceAmount
            | ApiError::InvoiceCanceled
            | ApiError::UnknownOperationId
            | ApiError::InsufficientBalance
            | ApiError::InvalidInvoiceDescription(_) => StatusCode::BAD_REQUEST,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(self)).into_response()
    }
}
