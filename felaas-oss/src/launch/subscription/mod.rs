use anyhow::Result;
use fedimint_client::OperationId;
use postgres_from_row::FromRow;
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::amount::Amount;
use crate::common::{ChatUserId, SubscriptionStatus};
use crate::wallet::core::Invoice;

pub mod api;
pub mod db;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PlanType {
    V0 {
        name: String,
        num_fedimints: u8,
        num_ogs: u8,
        price: Amount,
        renew_months: u8,
    },
}

impl PlanType {
    pub fn price(&self) -> Amount {
        match self {
            PlanType::V0 { price, .. } => *price,
        }
    }
    pub fn name(&self) -> String {
        match self {
            PlanType::V0 { name, .. } => name.clone(),
        }
    }
    pub fn num_fedimints(&self) -> u8 {
        match self {
            PlanType::V0 { num_fedimints, .. } => *num_fedimints,
        }
    }
    pub fn num_ogs(&self) -> u8 {
        match self {
            PlanType::V0 { num_ogs, .. } => *num_ogs,
        }
    }
}

impl std::str::FromStr for PlanType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(serde_json::from_str(s)?)
    }
}

impl ToSql for PlanType {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        serde_json::to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <serde_json::Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for PlanType {
    fn from_sql(
        ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(serde_json::from_value(serde_json::Value::from_sql(
            ty, raw,
        )?)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <serde_json::Value as FromSql>::accepts(ty)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SubscriptionCancellationReason {
    SubscriptionPaymentError(SubscriptionPaymentError),
}

impl ToSql for SubscriptionCancellationReason {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <serde_json::Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for SubscriptionCancellationReason {
    fn from_sql(
        ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(serde_json::from_value(serde_json::Value::from_sql(
            ty, raw,
        )?)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <serde_json::Value as FromSql>::accepts(ty)
    }
}

/// Represents a user's subscription to a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Subscription {
    pub user_id: ChatUserId,
    pub plan: PlanType,
    pub status: SubscriptionStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub activated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_updated_at: chrono::DateTime<chrono::Utc>,
    pub cancelled_reason: Option<SubscriptionCancellationReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentRequest {
    pub invoice: Invoice,
    pub operation_id: OperationId,
}

impl<'a> FromSql<'a> for PaymentRequest {
    fn from_sql(
        ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(serde_json::from_value(serde_json::Value::from_sql(
            ty, raw,
        )?)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <serde_json::Value as FromSql>::accepts(ty)
    }
}

impl ToSql for PaymentRequest {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <serde_json::Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InsufficientBalanceDetails {
    pub required: Amount,
    pub available: Amount,
}

impl ToSql for InsufficientBalanceDetails {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <serde_json::Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for InsufficientBalanceDetails {
    fn from_sql(
        ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(serde_json::from_value(serde_json::Value::from_sql(
            ty, raw,
        )?)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <serde_json::Value as FromSql>::accepts(ty)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SubscriptionPaymentError {
    InsufficientBalance(InsufficientBalanceDetails),
    InternalError(String),
}

impl ToSql for SubscriptionPaymentError {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <serde_json::Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for SubscriptionPaymentError {
    fn from_sql(
        ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(serde_json::from_value(serde_json::Value::from_sql(
            ty, raw,
        )?)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <serde_json::Value as FromSql>::accepts(ty)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SubscriptionPaymentInvoiceDetails {
    pub user_id: ChatUserId,
    pub plan_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct SubscriptionPayment {
    pub id: SubscriptionPaymentId,
    pub user_id: ChatUserId,
    pub plan: PlanType,
    pub amount: Amount,
    pub payment_request: PaymentRequest,
    pub preimage: Option<String>,
    pub failure_reason: Option<SubscriptionPaymentError>,
    pub status: SubscriptionPaymentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionPaymentId(Uuid);

impl std::str::FromStr for SubscriptionPaymentId {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::from_str(s)?))
    }
}

impl ToSql for SubscriptionPaymentId {
    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <Uuid as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        <Uuid as ToSql>::to_sql_checked(&self.0, ty, out)
    }

    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        <Uuid as ToSql>::to_sql(&self.0, ty, out)
    }
}

impl<'a> FromSql<'a> for SubscriptionPaymentId {
    fn from_sql(
        ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(Uuid::from_sql(ty, raw)?))
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <Uuid as FromSql>::accepts(ty)
    }
}

impl std::fmt::Display for SubscriptionPaymentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, strum::EnumString, strum::Display,
)]
pub enum SubscriptionPaymentStatus {
    Pending,
    Successful,
    Failed,
}

impl ToSql for SubscriptionPaymentStatus {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for SubscriptionPaymentStatus {
    fn from_sql(
        _ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        Ok(std::str::FromStr::from_str(s)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}
