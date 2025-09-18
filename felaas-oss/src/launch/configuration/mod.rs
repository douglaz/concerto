use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use fedimint_core::invite_code::InviteCode;
use postgres_from_row::FromRow;
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;
use uuid::Uuid;

use crate::amount::Amount;
use crate::common::{ChatUserId, Endpoint};

pub mod api;
pub mod db;

#[derive(
    Debug,
    Copy,
    Clone,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    strum::EnumString,
    strum::Display,
)]
pub enum FederationLaunchStatus {
    Requested,           // when requested, and talks to OG have started
    InProgress,          // talking to OGs or spinning up machines
    InfrastructureReady, // infrastructure deployed, endpoints set
    ReadyForDkg,         // when OG is filled out and ready for DKG
    Ready,               // finished DKG
}

impl<'a> FromSql<'a> for FederationLaunchStatus {
    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }

    fn from_sql(
        _ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        Ok(std::str::FromStr::from_str(s)?)
    }
}

impl ToSql for FederationLaunchStatus {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}

impl FederationLaunchStatus {
    pub fn validate_transition_to(
        &self,
        new_status: FederationLaunchStatus,
    ) -> Result<(), FederationLaunchError> {
        match (self, new_status) {
            (FederationLaunchStatus::Requested, FederationLaunchStatus::InProgress) => Ok(()),
            (FederationLaunchStatus::InProgress, FederationLaunchStatus::InfrastructureReady) => {
                Ok(())
            }
            (FederationLaunchStatus::InfrastructureReady, FederationLaunchStatus::ReadyForDkg) => {
                Ok(())
            }
            (FederationLaunchStatus::ReadyForDkg, FederationLaunchStatus::Ready) => Ok(()),
            (a, b) => Err(FederationLaunchError::InvalidStatusTransition(
                a.to_owned(),
                b.to_owned(),
            )),
        }
    }
}

#[derive(Debug, Error)]
pub enum FederationLaunchError {
    #[error("Invalid number of og/fedimints: ogs={0}, fedimints={1}")]
    InvalidNumOfGuardians(u8, u8),

    #[error("Invalid number of fedimints: {0}")]
    InvalidNumOfFedimints(u8),

    #[error("Launch id does not exists or it is in an invalid status")]
    LaunchIdNotFoundOrIsInInvalidStatus,

    #[error("Invalid status transition, {0} -> {1}")]
    InvalidStatusTransition(FederationLaunchStatus, FederationLaunchStatus),

    #[error("Database error: {0}")]
    DatabaseError(#[from] tokio_postgres::Error),

    #[error("Pool error: {0}")]
    DatabasePoolError(#[from] deadpool_postgres::PoolError),

    #[error("User does not have an active subscription")]
    NoActiveSubscription,

    #[error("Plan limit exceeded for fedimints: allowed={0}, requested_total={1}")]
    PlanLimitExceededFedimints(u16, u16),

    #[error("Plan limit exceeded for ogs: allowed={0}, requested_total={1}")]
    PlanLimitExceededOgs(u16, u16),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

/// An ID that identified a federation that is being set up
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FederationLaunchId(Uuid);

impl From<Uuid> for FederationLaunchId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl std::str::FromStr for FederationLaunchId {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::from_str(s)?))
    }
}

impl ToSql for FederationLaunchId {
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

impl<'a> FromSql<'a> for FederationLaunchId {
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

impl std::fmt::Display for FederationLaunchId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Liquidity configuration for a federation launch
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FederationLaunchLiquidity {
    pub status: FederationLaunchLiquidityStatus,
    pub gateway_liquidity: Amount,
    pub stability_provision: Amount,
}

/// Status of liquidity for a federation launch
#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    strum::EnumString,
    strum::Display,
)]
pub enum FederationLaunchLiquidityStatus {
    FundsRequested,
    Funded,
}

impl<'a> FromSql<'a> for FederationLaunchLiquidityStatus {
    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }

    fn from_sql(
        _ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        Ok(std::str::FromStr::from_str(s)?)
    }
}

impl ToSql for FederationLaunchLiquidityStatus {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InviteCodeSql(InviteCode);

impl From<InviteCode> for InviteCodeSql {
    fn from(code: InviteCode) -> Self {
        Self(code)
    }
}

impl From<InviteCodeSql> for InviteCode {
    fn from(sql_code: InviteCodeSql) -> Self {
        sql_code.0
    }
}

impl std::str::FromStr for InviteCodeSql {
    type Err = <InviteCode as std::str::FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

impl std::fmt::Display for InviteCodeSql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'a> FromSql<'a> for InviteCodeSql {
    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }

    fn from_sql(
        _ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        Ok(std::str::FromStr::from_str(s)?)
    }
}

impl ToSql for InviteCodeSql {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}

/// Configuration for a launch of a federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationLaunchConfiguration {
    pub launch_id: FederationLaunchId,
    pub name: String,
    pub status: FederationLaunchStatus,
    pub num_guardians: u8,
    pub num_ogs: u8,
    pub num_fedimints: u8,
    pub user_id: ChatUserId,
    pub fedimint_user_ids: Vec<ChatUserId>,
    pub og_user_ids: Vec<ChatUserId>,
    pub guardians_configurations: Vec<DkgGuardianConfiguration>,
    pub liquidity: Option<FederationLaunchLiquidity>,
    pub invite_code: Option<InviteCode>,
    pub created_at: DateTime<Utc>,
    pub last_updated_at: DateTime<Utc>,
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GuardianRole {
    Leader,
    Follower,
}

/// Configuration for a guardian
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DkgGuardianConfiguration {
    pub api_endpoint: Endpoint,
    pub admin_ui_endpoint: Option<Endpoint>,
    pub role: GuardianRole,
    pub user_id: ChatUserId,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
struct FederationLaunchConfigurationRow {
    launch_id: FederationLaunchId,
    name: String,
    status: FederationLaunchStatus,
    num_guardians: i32,
    num_ogs: i32,
    num_fedimints: i32,
    user_id: ChatUserId,
    fedimint_user_ids: Vec<ChatUserId>,
    og_user_ids: Vec<ChatUserId>,
    guardian_endpoints: Vec<Endpoint>,
    admin_ui_endpoints: Vec<Endpoint>,
    liquidity: Option<serde_json::Value>,
    invite_code: Option<InviteCodeSql>,
    created_at: DateTime<Utc>,
    last_updated_at: DateTime<Utc>,
}

impl TryFrom<FederationLaunchConfigurationRow> for FederationLaunchConfiguration {
    type Error = anyhow::Error;

    fn try_from(value: FederationLaunchConfigurationRow) -> anyhow::Result<Self> {
        let users_ids: Vec<_> = value
            .fedimint_user_ids
            .iter()
            .chain(value.og_user_ids.iter())
            .cloned()
            .collect();
        ensure!(
            value.admin_ui_endpoints.len() <= value.guardian_endpoints.len(),
            "Too many admin ui endpoints or too little guardian endpoints"
        );

        let admin_ui_endpoints = (0..value.guardian_endpoints.len())
            .map(|index| value.admin_ui_endpoints.get(index))
            .collect::<Vec<_>>();
        let guardians_configurations: Vec<DkgGuardianConfiguration> = value
            .guardian_endpoints
            .into_iter()
            .zip(admin_ui_endpoints)
            .zip(users_ids)
            .enumerate()
            .map(
                |(index, ((api_endpoint, admin_ui_endpoint), user_id))| DkgGuardianConfiguration {
                    api_endpoint,
                    admin_ui_endpoint: admin_ui_endpoint.cloned(),
                    role: if index == 0 {
                        GuardianRole::Leader
                    } else {
                        GuardianRole::Follower
                    },
                    user_id,
                },
            )
            .collect();

        let liquidity = value
            .liquidity
            .map(serde_json::from_value::<FederationLaunchLiquidity>)
            .transpose()?;

        let invite_code = value.invite_code.map(|sql_code| sql_code.into());

        Ok(FederationLaunchConfiguration {
            launch_id: value.launch_id,
            name: value.name,
            status: value.status,
            num_guardians: value.num_guardians.try_into()?,
            num_ogs: value.num_ogs.try_into()?,
            num_fedimints: value.num_fedimints.try_into()?,
            user_id: value.user_id,
            fedimint_user_ids: value.fedimint_user_ids,
            og_user_ids: value.og_user_ids,
            guardians_configurations,
            liquidity,
            invite_code,
            created_at: value.created_at,
            last_updated_at: value.last_updated_at,
        })
    }
}
