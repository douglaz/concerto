use anyhow::Result;
use fedimint_core::invite_code::InviteCode;
use postgres_from_row::FromRow;
use postgres_types::ToSql;
use tracing::{debug, error, info, warn};

use crate::PgPool;
use crate::common::{ChatUserId, Endpoint, SubscriptionStatus};
use crate::launch::configuration::{
    FederationLaunchConfiguration, FederationLaunchConfigurationRow, FederationLaunchError,
    FederationLaunchId, FederationLaunchLiquidity, FederationLaunchLiquidityStatus,
    FederationLaunchStatus, InviteCodeSql,
}; // ensure this matches your project dependencies

const FEDERATION_LAUNCH_CONFIGURATION: &str = "federation_launch_configuration";

#[derive(Clone)]
pub struct FederationLauncherDB {
    pool: PgPool,
    schema: String,
}

impl FederationLauncherDB {
    pub fn new(pool: PgPool, schema: String) -> Self {
        Self { pool, schema }
    }

    pub async fn create_table_if_not_exists(&self) -> Result<()> {
        let client = self.pool.get().await?;
        let schema = &self.schema;

        if let Err(e) = client
            .batch_execute(&format!(r#"CREATE SCHEMA IF NOT EXISTS {schema};"#))
            .await
        {
            // ignore error if schema already exists
            if e.code() != Some(&tokio_postgres::error::SqlState::DUPLICATE_SCHEMA)
                && e.code() != Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
            {
                error!(?e, %schema, "Failed to create schema");
                return Err(e.into());
            }
        }

        client
            .batch_execute(&format!(
                r#"
                CREATE TABLE IF NOT EXISTS {schema}.{FEDERATION_LAUNCH_CONFIGURATION} (
                    launch_id UUID PRIMARY KEY,
                    name TEXT NOT NULL,
                    status TEXT NOT NULL,
                    num_guardians INTEGER NOT NULL,
                    num_ogs INTEGER NOT NULL,
                    num_fedimints INTEGER NOT NULL,
                    user_id TEXT NOT NULL,
                    fedimint_user_ids TEXT[] NOT NULL,
                    og_user_ids TEXT[] NOT NULL,
                    guardian_endpoints TEXT[] NOT NULL,
                    admin_ui_endpoints TEXT[] NOT NULL,
                    liquidity JSONB,
                    invite_code TEXT,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                );
                ALTER TABLE {schema}.{FEDERATION_LAUNCH_CONFIGURATION} ADD COLUMN IF NOT EXISTS admin_ui_endpoints TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];
                ALTER TABLE {schema}.{FEDERATION_LAUNCH_CONFIGURATION} ADD COLUMN IF NOT EXISTS liquidity JSONB;
                ALTER TABLE {schema}.{FEDERATION_LAUNCH_CONFIGURATION} ADD COLUMN IF NOT EXISTS invite_code TEXT;
                CREATE INDEX IF NOT EXISTS idx_{schema}_{FEDERATION_LAUNCH_CONFIGURATION}_user_id
                ON {schema}.{FEDERATION_LAUNCH_CONFIGURATION} (user_id);
                CREATE INDEX IF NOT EXISTS idx_{schema}_{FEDERATION_LAUNCH_CONFIGURATION}_status
                ON {schema}.{FEDERATION_LAUNCH_CONFIGURATION} (status);
                CREATE INDEX IF NOT EXISTS idx_{schema}_{FEDERATION_LAUNCH_CONFIGURATION}_created_at
                ON {schema}.{FEDERATION_LAUNCH_CONFIGURATION} (created_at);
                CREATE INDEX IF NOT EXISTS idx_{schema}_{FEDERATION_LAUNCH_CONFIGURATION}_last_updated_at
                ON {schema}.{FEDERATION_LAUNCH_CONFIGURATION} (last_updated_at);
            "#
            ))
            .await?;
        Ok(())
    }

    pub async fn search_launch_configurations(
        &self,
        fedimint_user_ids: &[ChatUserId],
        og_user_ids: &[ChatUserId],
        status: &[FederationLaunchStatus],
        liquidity_status: Option<&FederationLaunchLiquidityStatus>,
    ) -> Result<Vec<FederationLaunchConfiguration>, FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        search_launch_configurations(
            &self.schema,
            &tx,
            fedimint_user_ids,
            og_user_ids,
            status,
            liquidity_status,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_standard_federation(
        &self,
        user_id: ChatUserId,
        name: String,
        num_fedimints: u8,
        num_ogs: u8,
        num_guardians: u8,
        fedimint_user_ids: Vec<ChatUserId>,
        og_user_ids: Vec<ChatUserId>,
    ) -> Result<FederationLaunchConfiguration, FederationLaunchError> {
        let schema = &self.schema;
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;

        // We suppose get_current_subscription will lock the user so we don't have
        // concurrency issues here
        let subscription =
            crate::launch::subscription::db::get_current_subscription(schema, &tx, &user_id)
                .await
                .map_err(|e| FederationLaunchError::Other(e.into()))?;

        let subscription = match subscription {
            Some(subscription) => subscription,
            None => {
                let err = FederationLaunchError::NoActiveSubscription;
                warn!(
                    ?err,
                    ?subscription,
                    "User tried to create federation without subscription"
                );
                return Err(err);
            }
        };

        // In future we may have other "active" statuses, so this match should be
        // exhaustive
        match subscription.status {
            SubscriptionStatus::Active => {
                debug!(
                    ?subscription,
                    "Will try to create a federation for an user with active subscription"
                );
            }
            SubscriptionStatus::Cancelled | SubscriptionStatus::PendingInitialActivation => {
                let err = FederationLaunchError::NoActiveSubscription;
                warn!(
                    ?err,
                    ?subscription,
                    "User tried to create federation without active subscription"
                );
                return Err(err);
            }
        };

        let (allowed_fedimints, allowed_ogs) = match subscription.plan {
            crate::launch::subscription::PlanType::V0 {
                num_fedimints,
                num_ogs,
                ..
            } => (num_fedimints, num_ogs),
        };
        let query = format!(
            r#"
                SELECT * FROM {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                WHERE user_id = $1
            "#,
        );
        let statement = tx.prepare_cached(&query).await?;
        let existing_configs = tx
            .query(&statement, &[&user_id])
            .await?
            .iter()
            .map(|row| {
                FederationLaunchConfigurationRow::try_from_row(row).map_err(anyhow::Error::new)
            })
            .collect::<Result<Vec<_>>>()?;

        let existing_fedimints: u16 = existing_configs
            .iter()
            .map(|c| c.num_fedimints as u16)
            .sum();
        let existing_ogs: u16 = existing_configs.iter().map(|c| c.num_ogs as u16).sum();

        let requested_total_fedimints = existing_fedimints + num_fedimints as u16;
        let requested_total_ogs = existing_ogs + num_ogs as u16;

        if requested_total_fedimints > allowed_fedimints as u16 {
            let err = FederationLaunchError::PlanLimitExceededFedimints(
                allowed_fedimints.into(),
                requested_total_fedimints,
            );
            warn!(
                ?err,
                "Cannot create federation config, fedimint limit exceeded"
            );
            return Err(err);
        }

        if requested_total_ogs > allowed_ogs as u16 {
            let err = FederationLaunchError::PlanLimitExceededOgs(
                allowed_ogs.into(),
                requested_total_ogs,
            );
            warn!(?err, "Cannot create federation config, og limit exceeded");
            return Err(err);
        }

        if num_guardians > num_ogs + num_fedimints {
            let err = FederationLaunchError::InvalidNumOfGuardians(num_ogs, num_fedimints);
            warn!(
                ?err,
                msg = %err,
                "Cannot create federation config with this arguments"
            );
            return Err(err);
        };

        if num_fedimints == 0 {
            let err = FederationLaunchError::InvalidNumOfFedimints(num_fedimints);
            warn!(
                ?err,
                msg = %err,
                "Cannot create federation config with this arguments"
            );
            return Err(err);
        };

        let launch_id = FederationLaunchId(uuid::Uuid::new_v4());

        let query = format!(
            r#"
                INSERT INTO {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                    (launch_id, name, status, num_guardians, num_ogs, num_fedimints, user_id, fedimint_user_ids, og_user_ids, guardian_endpoints, admin_ui_endpoints, invite_code)
                VALUES
                    ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                RETURNING
                    *
            "#
        );

        let statement = tx.prepare_cached(&query).await?;

        let guardian_endpoints: Vec<&str> = vec![];
        let admin_ui_endpoints: Vec<&str> = vec![];
        let row = tx
            .query_one(
                &statement,
                &[
                    &launch_id,
                    &name,
                    &FederationLaunchStatus::Requested,
                    &i32::from(num_guardians),
                    &i32::from(num_ogs),
                    &i32::from(num_fedimints),
                    &user_id,
                    &fedimint_user_ids,
                    &og_user_ids,
                    &guardian_endpoints,
                    &admin_ui_endpoints,
                    &Option::<String>::None,
                ],
            )
            .await?;

        let federation_launch_configuration: FederationLaunchConfiguration =
            FederationLaunchConfigurationRow::try_from_row(&row)?.try_into()?;
        tx.commit().await?;
        info!(
            ?federation_launch_configuration,
            "Federation launch configuration created"
        );
        Ok(federation_launch_configuration)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_fiat_federation(
        &self,
        user_id: ChatUserId,
        name: String,
        num_fedimints: u8,
        num_ogs: u8,
        num_guardians: u8,
        fedimint_user_ids: Vec<ChatUserId>,
        og_user_ids: Vec<ChatUserId>,
    ) -> Result<FederationLaunchConfiguration, FederationLaunchError> {
        let schema = &self.schema;
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;

        // We suppose get_current_subscription will lock the user so we don't have
        // concurrency issues here
        let subscription =
            crate::launch::subscription::db::get_current_subscription(schema, &tx, &user_id)
                .await
                .map_err(|e| FederationLaunchError::Other(e.into()))?;

        let subscription = match subscription {
            Some(subscription) => subscription,
            None => {
                let err = FederationLaunchError::NoActiveSubscription;
                warn!(
                    ?err,
                    ?subscription,
                    "User tried to create federation without subscription"
                );
                return Err(err);
            }
        };

        // In future we may have other "active" statuses, so this match should be
        // exhaustive
        match subscription.status {
            SubscriptionStatus::Active => {
                debug!(
                    ?subscription,
                    "Will try to create a federation for an user with active subscription"
                );
            }
            SubscriptionStatus::Cancelled | SubscriptionStatus::PendingInitialActivation => {
                let err = FederationLaunchError::NoActiveSubscription;
                warn!(
                    ?err,
                    ?subscription,
                    "User tried to create federation without active subscription"
                );
                return Err(err);
            }
        };

        let allowed_fedimints = subscription.plan.num_fedimints();
        let allowed_ogs = subscription.plan.num_ogs();

        let query = format!(
            r#"
                SELECT * FROM {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                WHERE user_id = $1
            "#,
        );
        let statement = tx.prepare_cached(&query).await?;
        let existing_configs = tx
            .query(&statement, &[&user_id])
            .await?
            .iter()
            .map(|row| {
                FederationLaunchConfigurationRow::try_from_row(row).map_err(anyhow::Error::new)
            })
            .collect::<Result<Vec<_>>>()?;

        let existing_fedimints: u16 = existing_configs
            .iter()
            .map(|c| c.num_fedimints as u16)
            .sum();
        let existing_ogs: u16 = existing_configs.iter().map(|c| c.num_ogs as u16).sum();

        let requested_total_fedimints = existing_fedimints + num_fedimints as u16;
        let requested_total_ogs = existing_ogs + num_ogs as u16;

        if requested_total_fedimints > allowed_fedimints as u16 {
            let err = FederationLaunchError::PlanLimitExceededFedimints(
                allowed_fedimints.into(),
                requested_total_fedimints,
            );
            warn!(
                ?err,
                "Cannot create federation config, fedimint limit exceeded"
            );
            return Err(err);
        }

        if requested_total_ogs > allowed_ogs as u16 {
            let err = FederationLaunchError::PlanLimitExceededOgs(
                allowed_ogs.into(),
                requested_total_ogs,
            );
            warn!(?err, "Cannot create federation config, og limit exceeded");
            return Err(err);
        }

        if num_guardians > num_ogs + num_fedimints {
            let err = FederationLaunchError::InvalidNumOfGuardians(num_ogs, num_fedimints);
            warn!(
                ?err,
                msg = %err,
                "Cannot create federation config with this arguments"
            );
            return Err(err);
        };

        if num_fedimints == 0 {
            let err = FederationLaunchError::InvalidNumOfFedimints(num_fedimints);
            warn!(
                ?err,
                msg = %err,
                "Cannot create federation config with this arguments"
            );
            return Err(err);
        };

        let launch_id = FederationLaunchId(uuid::Uuid::new_v4());

        let query = format!(
            r#"
                INSERT INTO {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                    (launch_id, name, status, num_guardians, num_ogs, num_fedimints, user_id, fedimint_user_ids, og_user_ids, guardian_endpoints, admin_ui_endpoints, invite_code)
                VALUES
                    ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                RETURNING
                    *
            "#
        );

        let statement = tx.prepare_cached(&query).await?;

        let guardian_endpoints: Vec<&str> = vec![];
        let admin_ui_endpoints: Vec<&str> = vec![];
        let row = tx
            .query_one(
                &statement,
                &[
                    &launch_id,
                    &name,
                    &FederationLaunchStatus::Requested,
                    &i32::from(num_guardians),
                    &i32::from(num_ogs),
                    &i32::from(num_fedimints),
                    &user_id,
                    &fedimint_user_ids,
                    &og_user_ids,
                    &guardian_endpoints,
                    &admin_ui_endpoints,
                    &Option::<String>::None,
                ],
            )
            .await?;

        let federation_launch_configuration: FederationLaunchConfiguration =
            FederationLaunchConfigurationRow::try_from_row(&row)?.try_into()?;
        tx.commit().await?;
        info!(
            ?federation_launch_configuration,
            "Federation launch configuration created"
        );
        Ok(federation_launch_configuration)
    }

    /// Create a test fiat federation for staging environments only
    /// This bypasses all subscription checks and directly creates a federation
    /// in Requested status
    #[allow(clippy::too_many_arguments)]
    pub async fn create_test_fiat_federation(
        &self,
        user_id: ChatUserId,
        name: String,
        num_fedimints: u8,
        num_ogs: u8,
        num_guardians: u8,
        fedimint_user_ids: Vec<ChatUserId>,
        og_user_ids: Vec<ChatUserId>,
    ) -> Result<FederationLaunchConfiguration, FederationLaunchError> {
        let schema = &self.schema;
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;

        // Basic validation only - no subscription checks
        if num_guardians != num_ogs + num_fedimints {
            let err = FederationLaunchError::InvalidNumOfGuardians(num_ogs, num_fedimints);
            warn!(?err, "Invalid guardian count for test federation");
            return Err(err);
        }

        if num_fedimints == 0 {
            let err = FederationLaunchError::InvalidNumOfFedimints(num_fedimints);
            warn!(?err, "Invalid fedimint count for test federation");
            return Err(err);
        }

        let launch_id = FederationLaunchId(uuid::Uuid::new_v4());

        let query = format!(
            r#"
                INSERT INTO {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                    (launch_id, name, status, num_guardians, num_ogs, num_fedimints, user_id, fedimint_user_ids, og_user_ids, guardian_endpoints, admin_ui_endpoints)
                VALUES
                    ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                RETURNING
                    *
            "#
        );

        let statement = tx.prepare_cached(&query).await?;

        let guardian_endpoints: Vec<&str> = vec![];
        let admin_ui_endpoints: Vec<&str> = vec![];
        let row = tx
            .query_one(
                &statement,
                &[
                    &launch_id,
                    &name,
                    &FederationLaunchStatus::Requested,
                    &i32::from(num_guardians),
                    &i32::from(num_ogs),
                    &i32::from(num_fedimints),
                    &user_id,
                    &fedimint_user_ids,
                    &og_user_ids,
                    &guardian_endpoints,
                    &admin_ui_endpoints,
                ],
            )
            .await?;

        let federation_launch_configuration: FederationLaunchConfiguration =
            FederationLaunchConfigurationRow::try_from_row(&row)?.try_into()?;
        tx.commit().await?;
        info!(
            ?federation_launch_configuration,
            "Test federation launch configuration created (staging mode)"
        );
        Ok(federation_launch_configuration)
    }

    pub async fn get_federation_launch_configuration(
        &self,
        launch_id: &FederationLaunchId,
    ) -> Result<Option<FederationLaunchConfiguration>, FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;
        let tx = client.transaction().await?;
        get_federation_launch_configuration(schema, &tx, launch_id).await
    }

    pub async fn set_ogs(
        &self,
        launch_id: FederationLaunchId,
        og_user_ids: Vec<ChatUserId>,
    ) -> Result<(), FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;

        let tx = client.transaction().await?;

        // Only allow setting OGs when infrastructure is ready
        set_ogs_internal(
            &tx,
            schema,
            &launch_id,
            &og_user_ids,
            &[FederationLaunchStatus::InfrastructureReady],
        )
        .await?;

        tx.commit().await?;

        info!(?launch_id, ?og_user_ids, "OGs updated successfully");
        Ok(())
    }

    /// Set OGs and transition to ReadyForDKG in a single transaction
    pub async fn set_ogs_and_transition_to_ready_for_dkg(
        &self,
        launch_id: FederationLaunchId,
        og_user_ids: Vec<ChatUserId>,
    ) -> Result<(), FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;

        let tx = client.transaction().await?;

        // Use the internal helper to set OGs (validates InfrastructureReady status)
        set_ogs_internal(
            &tx,
            schema,
            &launch_id,
            &og_user_ids,
            &[FederationLaunchStatus::InfrastructureReady],
        )
        .await?;

        // Update status to ReadyForDkg
        let status_statement = tx
            .prepare_cached(&format!(
                r#"
                    UPDATE {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                    SET status = $1, last_updated_at = NOW()
                    WHERE launch_id = $2 AND status = $3
                "#
            ))
            .await?;

        let rows_status = tx
            .execute(
                &status_statement,
                &[
                    &FederationLaunchStatus::ReadyForDkg,
                    &launch_id,
                    &FederationLaunchStatus::InfrastructureReady,
                ],
            )
            .await?;

        if rows_status == 0 {
            let err = FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus;
            warn!(?err, "Failed to update status to ReadyForDkg");
            return Err(err);
        }

        // Commit the transaction
        tx.commit().await?;

        info!(
            ?launch_id,
            ?og_user_ids,
            "OGs set and transitioned to ReadyForDkg successfully"
        );
        Ok(())
    }

    pub async fn set_guardian_endpoint(
        &self,
        launch_id: FederationLaunchId,
        guardian_endpoints: Vec<Endpoint>,
        admin_ui_endpoints: Vec<Endpoint>,
    ) -> Result<(), FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;
        let tx = client.transaction().await?;

        // Get the plan and check how many guardian we want
        let launch_configuration =
            match get_federation_launch_configuration(schema, &tx, &launch_id).await? {
                Some(launch_configuration) => launch_configuration,
                None => {
                    let err = FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus;
                    warn!(?err, "Failed to set guardian endpoint");
                    return Err(err);
                }
            };

        let required_endpoints = launch_configuration.num_guardians;

        if guardian_endpoints.len() != required_endpoints as usize {
            let err = FederationLaunchError::Other(anyhow::anyhow!(
                "Invalid number of guardian endpoints: got {actual} expected {required_endpoints}",
                actual = guardian_endpoints.len()
            ));
            warn!(?err, required_endpoints, "Failed to set guardian endpoint");
            return Err(err);
        }

        if !admin_ui_endpoints.is_empty() && admin_ui_endpoints.len() != required_endpoints as usize
        {
            let err = FederationLaunchError::Other(anyhow::anyhow!(
                "Invalid number of admin UI endpoints: got {actual} expected {required_endpoints}",
                actual = admin_ui_endpoints.len()
            ));
            warn!(?err, required_endpoints, "Failed to set admin UI endpoint");
            return Err(err);
        }

        let update_query = format!(
            r#"
                UPDATE {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                SET guardian_endpoints = $1, admin_ui_endpoints = $2, last_updated_at = NOW()
                WHERE launch_id = $3 AND status = ANY($4)
            "#
        );

        let update_statement = tx.prepare_cached(&update_query).await?;

        let rows_updated = tx
            .execute(
                &update_statement,
                &[
                    &guardian_endpoints,
                    &admin_ui_endpoints,
                    &launch_id,
                    &vec![
                        FederationLaunchStatus::Requested,
                        FederationLaunchStatus::InProgress,
                    ],
                ],
            )
            .await?;

        if rows_updated == 0 {
            let err = FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus;
            warn!(?err, "Failed to update guardian endpoints");
            return Err(err);
        }
        tx.commit().await?;

        info!(
            ?launch_id,
            ?guardian_endpoints,
            ?admin_ui_endpoints,
            "Guardian endpoints updated"
        );
        Ok(())
    }

    pub async fn update_status(
        &self,
        launch_id: FederationLaunchId,
        new_status: FederationLaunchStatus,
    ) -> Result<(), FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;

        // Begin a transaction
        let tx = client.transaction().await?;

        let query = format!(
            r#"
                SELECT status
                FROM {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                WHERE launch_id = $1
                FOR UPDATE
            "#
        );

        let statement = tx.prepare_cached(&query).await?;

        let row = tx.query_opt(&statement, &[&launch_id]).await?;

        let row = match row {
            Some(row) => row,
            None => {
                let err = FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus;
                warn!(
                    ?launch_id,
                    ?err,
                    "Failed to update federation launch status, launch configuration not found"
                );
                return Err(err);
            }
        };

        let current_status: FederationLaunchStatus = row.try_get("status")?;

        // Validate the transition
        if let Err(invalid_transition) = current_status.validate_transition_to(new_status) {
            warn!(
                ?current_status,
                ?new_status,
                "Status transition is not valid!"
            );
            return Err(invalid_transition);
        }

        let query = format!(
            r#"
                UPDATE {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                SET status = $1, last_updated_at = NOW()
                WHERE launch_id = $2
            "#
        );

        let statement = tx.prepare_cached(&query).await?;

        let rows_affected = tx.execute(&statement, &[&new_status, &launch_id]).await?;

        if rows_affected == 0 {
            return Err(anyhow::anyhow!("No launch found for launch id: {launch_id:?}").into());
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn override_status(
        &self,
        launch_id: FederationLaunchId,
        old_status: FederationLaunchStatus,
        new_status: FederationLaunchStatus,
    ) -> Result<(), FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;

        // Begin a transaction
        let tx = client.transaction().await?;

        let query = format!(
            r#"
                UPDATE {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                SET status = $1, last_updated_at = NOW()
                WHERE status = $2 AND launch_id = $3
            "#
        );

        let statement = tx.prepare_cached(&query).await?;

        let rows_affected = tx
            .execute(&statement, &[&new_status, &old_status, &launch_id])
            .await?;

        if rows_affected == 0 {
            return Err(anyhow::anyhow!(
                "No launch found for launch id: {launch_id:?} with status {old_status:?}"
            )
            .into());
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn update_liquidity(
        &self,
        launch_id: FederationLaunchId,
        liquidity: FederationLaunchLiquidity,
    ) -> Result<(), FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;

        let tx = client.transaction().await?;

        // Serialize liquidity to JSON
        let liquidity_json =
            serde_json::to_value(&liquidity).map_err(|e| FederationLaunchError::Other(e.into()))?;

        let query = format!(
            r#"
                UPDATE {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                SET liquidity = $1, last_updated_at = NOW()
                WHERE launch_id = $2
            "#
        );

        let statement = tx.prepare_cached(&query).await?;

        let rows_affected = tx
            .execute(&statement, &[&liquidity_json, &launch_id])
            .await?;

        if rows_affected == 0 {
            return Err(FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus);
        }

        tx.commit().await?;
        info!(?launch_id, ?liquidity, "Updated federation liquidity");
        Ok(())
    }

    pub async fn set_invite_code(
        &self,
        launch_id: FederationLaunchId,
        invite_code: InviteCode,
    ) -> Result<(), FederationLaunchError> {
        let mut client = self.pool.get().await?;
        let schema = &self.schema;
        let tx = client.transaction().await?;

        let sql_invite_code = InviteCodeSql::from(invite_code.clone());
        let query = format!(
            r#"
                UPDATE {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                SET invite_code = $1, last_updated_at = NOW()
                WHERE launch_id = $2
            "#
        );
        let statement = tx.prepare_cached(&query).await?;
        let rows_affected = tx
            .execute(&statement, &[&sql_invite_code, &launch_id])
            .await?;

        if rows_affected == 0 {
            return Err(FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus);
        }

        tx.commit().await?;
        info!(?launch_id, ?invite_code, "Updated federation invite code");
        Ok(())
    }
}

/// Internal helper to set OGs within an existing transaction
async fn set_ogs_internal(
    tx: &deadpool_postgres::Transaction<'_>,
    schema: &str,
    launch_id: &FederationLaunchId,
    og_user_ids: &[ChatUserId],
    allowed_statuses: &[FederationLaunchStatus],
) -> Result<(), FederationLaunchError> {
    let statement = tx
        .prepare_cached(&format!(
            r#"
                UPDATE {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
                SET og_user_ids = $1, last_updated_at = NOW()
                WHERE launch_id = $2 AND status = ANY($3)
            "#
        ))
        .await?;

    let rows = tx
        .execute(&statement, &[&og_user_ids, &launch_id, &allowed_statuses])
        .await?;

    if rows == 0 {
        let err = FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus;
        warn!(?err, ?launch_id, "Failed to set OGs");
        return Err(err);
    }

    info!(?launch_id, ?og_user_ids, "OGs set successfully");
    Ok(())
}

pub async fn get_federation_launch_configuration(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    launch_id: &FederationLaunchId,
) -> Result<Option<FederationLaunchConfiguration>, FederationLaunchError> {
    let query = format!(
        r#"
            SELECT
                *
            FROM
                {schema}.{FEDERATION_LAUNCH_CONFIGURATION}
            WHERE
                launch_id = $1
            FOR UPDATE
        "#
    );

    let statement = tx.prepare_cached(&query).await?;

    let maybe_row = tx.query_opt(&statement, &[launch_id]).await?;

    match maybe_row {
        Some(row) => {
            let parsed = FederationLaunchConfigurationRow::try_from_row(&row)?;
            let launch_configuration: FederationLaunchConfiguration = parsed.try_into()?;
            debug!(
                ?launch_id,
                ?launch_configuration,
                "Got launch configuration for launch id"
            );
            Ok(Some(launch_configuration))
        }
        None => {
            debug!(
                ?launch_id,
                "No federation launch configuration found for this launch id"
            );
            Ok(None)
        }
    }
}

pub async fn search_launch_configurations(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    fedimint_user_ids: &[ChatUserId],
    og_user_ids: &[ChatUserId],
    status: &[FederationLaunchStatus],
    liquidity_status: Option<&FederationLaunchLiquidityStatus>,
) -> Result<Vec<FederationLaunchConfiguration>, FederationLaunchError> {
    if fedimint_user_ids.is_empty()
        && og_user_ids.is_empty()
        && status.is_empty()
        && liquidity_status.is_none()
    {
        return Ok(Default::default());
    }

    let mut filters: Vec<String> = vec![];
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![];

    if !fedimint_user_ids.is_empty() {
        params.push(&fedimint_user_ids);
        filters.push(format!("fedimint_user_ids @> ${len}", len = params.len()));
    }

    if !og_user_ids.is_empty() {
        params.push(&og_user_ids);
        filters.push(format!("og_user_ids @> ${len}", len = params.len()));
    }

    if !status.is_empty() {
        params.push(&status);
        filters.push(format!("status = ANY(${len})", len = params.len()));
    }

    if let Some(liquidity_status) = liquidity_status {
        params.push(liquidity_status);
        filters.push(format!("liquidity->>'status' = ${len}", len = params.len()));
    }

    let filter_clause = if !filters.is_empty() {
        format!(" WHERE {filters}", filters = filters.join(" AND "))
    } else {
        String::new()
    };

    let query = format!(
        "SELECT * FROM {schema}.{FEDERATION_LAUNCH_CONFIGURATION} {filter_clause} FOR UPDATE"
    );

    let launch_configurations = tx
        .query(&query, params.as_slice())
        .await?
        .iter()
        .map(|row| FederationLaunchConfigurationRow::try_from_row(row)?.try_into())
        .collect::<anyhow::Result<Vec<FederationLaunchConfiguration>>>()?;
    Ok(launch_configurations)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::str::FromStr;

    use anyhow::{Context, Result};
    use deadpool_postgres::{Config, Pool, Runtime};
    use tokio_postgres::NoTls;
    use uuid::Uuid;

    use super::*;
    use crate::common::mock_random_invoice;
    use crate::launch::fiat_subscription::db::{FiatSubscriptionDB, create_test_exchange_rates};
    use crate::launch::subscription::db::SubscriptionDB;
    use crate::wallet::core::FelaasWallet;
    use crate::{
        PGDATABASE, PGHOST, PGPASSWORD, PGPORT, PGUSER, fiat_daemon, initialize_logging,
        standard_daemon,
    };

    // Helper to create a test database pool
    async fn test_db_pool() -> Result<Pool> {
        initialize_logging();
        let pg_user = env::var(PGUSER).ok();
        let pg_pass = env::var(PGPASSWORD).ok();
        let pg_host = env::var(PGHOST).ok();
        let pg_db = env::var(PGDATABASE).ok();
        let pg_port = env::var(PGPORT).ok();

        println!(
            "[test_db_pool] PGUSER={pg_user:?} PGPASS={pg_pass:?} PGHOST={pg_host:?} PGPORT={pg_port:?} PGDATABASE={pg_db:?}"
        );

        let mut cfg = Config::new();
        cfg.user = pg_user;
        cfg.password = pg_pass;
        cfg.host = pg_host;
        cfg.dbname = pg_db;
        if let Some(port) = pg_port {
            if let Ok(port) = port.parse::<u16>() {
                cfg.port = Some(port);
            }
        }
        cfg.manager = Some(deadpool_postgres::ManagerConfig {
            recycling_method: deadpool_postgres::RecyclingMethod::Fast,
        });
        let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)?;

        Ok(pool)
    }

    async fn create_subscription_test_launch(
        schema: &str,
        pool: &Pool,
        wallet: &FelaasWallet,
        federation_db: &FederationLauncherDB,
        subscription_db: &SubscriptionDB,
    ) -> anyhow::Result<FederationLaunchConfiguration> {
        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let plan = crate::launch::subscription::PlanType::V0 {
            name: "FriendsWithoutBenefits".into(),
            num_fedimints: 1,
            num_ogs: 3,
            price: crate::amount::Amount::from_bitcoins(1),
            renew_months: 1,
        };

        let allow_internal_invoice = true;
        subscription_db
            .subscribe(wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        standard_daemon::try_process_pending_subscription(schema, pool, wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let crate::launch::subscription::PlanType::V0 {
            num_fedimints,
            num_ogs,
            name: _,
            price: _,
            renew_months: _,
        } = plan;

        // Create a federation using the create_federation method
        let name = String::from("GetConfigTest");
        let num_guardians = num_fedimints + num_ogs;
        let fedimint_user_ids = vec![user_id.clone()];
        let og_user_ids = vec![user_id.clone()];

        let federation_launch_configuration = federation_db
            .create_standard_federation(
                user_id.clone(),
                name.clone(),
                num_fedimints,
                num_ogs,
                num_guardians,
                fedimint_user_ids.clone(),
                og_user_ids.clone(),
            )
            .await?;

        let config = federation_db
            .get_federation_launch_configuration(&federation_launch_configuration.launch_id)
            .await?
            .context("Failed to get federation launch configuration")?;

        assert_eq!(config.launch_id, federation_launch_configuration.launch_id);
        assert_eq!(config.name, name);
        assert_eq!(config.status, FederationLaunchStatus::Requested);
        assert_eq!(config.num_fedimints, num_fedimints);
        assert_eq!(config.num_ogs, num_ogs);
        assert_eq!(config.num_guardians, num_guardians);
        assert_eq!(config.user_id, user_id);
        assert_eq!(config.fedimint_user_ids, fedimint_user_ids);
        assert_eq!(config.og_user_ids, og_user_ids);
        assert!(config.guardians_configurations.is_empty());
        Ok(config)
    }

    async fn create_fiat_subscription_test_launch(
        schema: &str,
        pool: &Pool,
        wallet: &FelaasWallet,
        federation_db: &FederationLauncherDB,
        subscription_db: &FiatSubscriptionDB,
    ) -> anyhow::Result<FederationLaunchConfiguration> {
        create_test_exchange_rates(subscription_db).await?;
        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let plan = crate::launch::fiat_subscription::db::get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();
        // First subscribe to a plan
        let allow_internal_invoice = true;
        let (_subscription, payment) = subscription_db
            .subscribe(wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        fiat_daemon::process_pending_subscription_payment(schema, pool, wallet, &payment)
            .await
            .context("Failed to process pending subscription")?;

        // Create a federation using the create_federation method
        let name = String::from("GetConfigTest");
        let num_guardians = plan.num_fedimints() + plan.num_ogs();
        let fedimint_user_ids = vec![user_id.clone()];
        let og_user_ids = vec![user_id.clone()];

        let federation_launch_configuration = federation_db
            .create_fiat_federation(
                user_id.clone(),
                name.clone(),
                plan.num_fedimints(),
                plan.num_ogs(),
                num_guardians,
                fedimint_user_ids.clone(),
                og_user_ids.clone(),
            )
            .await?;

        let config = federation_db
            .get_federation_launch_configuration(&federation_launch_configuration.launch_id)
            .await?
            .context("Failed to get federation launch configuration")?;

        assert_eq!(config.launch_id, federation_launch_configuration.launch_id);
        assert_eq!(config.name, name);
        assert_eq!(config.status, FederationLaunchStatus::Requested);
        assert_eq!(config.num_fedimints, plan.num_fedimints());
        assert_eq!(config.num_ogs, plan.num_ogs());
        assert_eq!(config.num_guardians, num_guardians);
        assert_eq!(config.user_id, user_id);
        assert_eq!(config.fedimint_user_ids, fedimint_user_ids);
        assert_eq!(config.og_user_ids, og_user_ids);
        assert!(config.guardians_configurations.is_empty());
        Ok(config)
    }

    #[tokio::test]
    async fn test_create_too_much_fedimints_subscription() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let plan = crate::launch::subscription::db::get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();
        // First subscribe to a plan
        let allow_internal_invoice = true;
        subscription_db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        standard_daemon::try_process_pending_subscription(&schema, &pool, &wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let crate::launch::subscription::PlanType::V0 {
            num_fedimints,
            num_ogs,
            name: _,
            price: _,
            renew_months: _,
        } = plan;

        let launch_id = db
            .create_standard_federation(
                user_id.clone(),
                String::from("test"),
                num_fedimints + 1,
                num_ogs,
                num_fedimints + num_ogs,
                vec![user_id.clone()],
                vec![user_id.clone()],
            )
            .await;
        assert!(matches!(
            launch_id,
            Err(FederationLaunchError::PlanLimitExceededFedimints(_, _))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_create_too_much_fedimints_fiat_subscription() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = FiatSubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        create_test_exchange_rates(&subscription_db).await?;

        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let plan = crate::launch::fiat_subscription::db::get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();
        // First subscribe to a plan
        let allow_internal_invoice = true;
        let (_subscription, payment) = subscription_db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        fiat_daemon::process_pending_subscription_payment(&schema, &pool, &wallet, &payment)
            .await
            .context("Failed to process pending subscription")?;

        let launch_id = db
            .create_fiat_federation(
                user_id.clone(),
                String::from("test"),
                plan.num_fedimints() + 1,
                plan.num_ogs(),
                plan.num_fedimints() + plan.num_ogs(),
                vec![user_id.clone()],
                vec![user_id.clone()],
            )
            .await;
        assert!(matches!(
            launch_id,
            Err(FederationLaunchError::PlanLimitExceededFedimints(_, _))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_create_too_much_fedimints_different_launches_subscription() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let plan = crate::launch::subscription::db::get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();
        // First subscribe to a plan
        let allow_internal_invoice = true;
        subscription_db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;

        standard_daemon::try_process_pending_subscription(&schema, &pool, &wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let crate::launch::subscription::PlanType::V0 {
            num_fedimints,
            num_ogs,
            name: _,
            price: _,
            renew_months: _,
        } = plan;

        // First launch should succeed
        let launch_id = db
            .create_standard_federation(
                user_id.clone(),
                String::from("test"),
                num_fedimints,
                num_ogs,
                num_fedimints + num_ogs,
                vec![user_id.clone()],
                vec![user_id.clone()],
            )
            .await;
        assert!(launch_id.is_ok());

        // Second launch should fail
        let launch_id = db
            .create_standard_federation(
                user_id.clone(),
                String::from("test"),
                num_fedimints + num_ogs,
                num_ogs,
                num_fedimints + num_ogs,
                vec![user_id.clone()],
                vec![user_id.clone()],
            )
            .await;
        assert!(matches!(
            launch_id,
            Err(FederationLaunchError::PlanLimitExceededFedimints(_, _))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_create_too_much_fedimints_different_launches_fiat_subscription() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = FiatSubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        create_test_exchange_rates(&subscription_db).await?;
        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let plan = crate::launch::fiat_subscription::db::get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();
        // First subscribe to a plan
        let allow_internal_invoice = true;
        let (_subscription, payment) = subscription_db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        fiat_daemon::process_pending_subscription_payment(&schema, &pool, &wallet, &payment)
            .await
            .context("Failed to process pending subscription")?;

        // First launch should succeed
        let launch_id = db
            .create_fiat_federation(
                user_id.clone(),
                String::from("test"),
                plan.num_fedimints(),
                plan.num_ogs(),
                plan.num_fedimints() + plan.num_ogs(),
                vec![user_id.clone()],
                vec![user_id.clone()],
            )
            .await;
        assert!(launch_id.is_ok());

        // Second launch should fail
        let launch_id = db
            .create_fiat_federation(
                user_id.clone(),
                String::from("test"),
                plan.num_fedimints() + plan.num_ogs(),
                plan.num_ogs(),
                plan.num_fedimints() + plan.num_ogs(),
                vec![user_id.clone()],
                vec![user_id.clone()],
            )
            .await;
        assert!(matches!(
            launch_id,
            Err(FederationLaunchError::PlanLimitExceededFedimints(_, _))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_update_status_success() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        let new_status = FederationLaunchStatus::InProgress;
        // from Requested -> InProgress
        db.update_status(launch_config.launch_id, new_status)
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_update_status_invalid_transition() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        let new_status = FederationLaunchStatus::Ready;

        let result = db.update_status(launch_config.launch_id, new_status).await;
        assert!(matches!(
            result,
            Err(FederationLaunchError::InvalidStatusTransition(_, _))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_set_ogs_success() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        // Transition to InfrastructureReady first
        db.update_status(
            launch_config.launch_id.clone(),
            FederationLaunchStatus::InProgress,
        )
        .await?;
        db.update_status(
            launch_config.launch_id.clone(),
            FederationLaunchStatus::InfrastructureReady,
        )
        .await?;

        let og_user_ids = vec![
            ChatUserId::from("og1"),
            ChatUserId::from("og2"),
            ChatUserId::from("og3"),
        ];

        db.set_ogs(launch_config.launch_id, og_user_ids).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_set_guardians_conf_success() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        let guardian_endpoints = vec![
            Endpoint::from_str("http://example1.com")?,
            Endpoint::from_str("http://example2.com")?,
            Endpoint::from_str("http://example3.com")?,
            Endpoint::from_str("http://example4.com")?,
        ];
        let admin_ui_endpoints = vec![
            Endpoint::from_str("http://guardian-ui-example1.com")?,
            Endpoint::from_str("http://guardian-ui-example2.com")?,
            Endpoint::from_str("http://guardian-ui-example3.com")?,
            Endpoint::from_str("http://guardian-ui-example4.com")?,
        ];

        db.set_guardian_endpoint(
            launch_config.launch_id,
            guardian_endpoints,
            admin_ui_endpoints,
        )
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_set_guardians_conf_failure_invalid_number_of_endpoints() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        let guardian_endpoints = vec![
            Endpoint::from_str("http://example1.com")?,
            Endpoint::from_str("http://example2.com")?,
        ];
        let admin_ui_endpoints = vec![
            Endpoint::from_str("http://guardian-ui-example1.com")?,
            Endpoint::from_str("http://guardian-ui-example2.com")?,
        ];

        assert!(
            db.set_guardian_endpoint(
                launch_config.launch_id,
                guardian_endpoints,
                admin_ui_endpoints,
            )
            .await
            .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_create_get_federation_launch_configuration_success_subscription() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;
        assert_eq!(launch_config.status, FederationLaunchStatus::Requested);
        Ok(())
    }

    #[tokio::test]
    async fn test_create_get_federation_launch_configuration_success_fiat_subscription()
    -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = FiatSubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_fiat_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db)
                .await?;
        assert_eq!(launch_config.status, FederationLaunchStatus::Requested);
        Ok(())
    }

    #[tokio::test]
    async fn test_create_get_federation_launch_configuration_not_found_subscription() -> Result<()>
    {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let random_id = FederationLaunchId::from_str("00000000-0000-0000-0000-000000000000")?;
        let config_opt = db.get_federation_launch_configuration(&random_id).await?;
        assert!(config_opt.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_create_get_federation_launch_configuration_not_found_fiat_subscription()
    -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = FiatSubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let random_id = FederationLaunchId::from_str("00000000-0000-0000-0000-000000000000")?;
        let config_opt = db.get_federation_launch_configuration(&random_id).await?;
        assert!(config_opt.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_update_liquidity_success() -> Result<()> {
        use crate::amount::Amount;
        use crate::launch::configuration::{
            FederationLaunchLiquidity, FederationLaunchLiquidityStatus,
        };

        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        let liquidity = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::FundsRequested,
            gateway_liquidity: Amount::from_sats(100_000),
            stability_provision: Amount::from_sats(50_000),
        };

        // Update liquidity
        db.update_liquidity(launch_config.launch_id.clone(), liquidity.clone())
            .await?;

        // Verify the update
        let updated_config = db
            .get_federation_launch_configuration(&launch_config.launch_id)
            .await?
            .context("Failed to get federation launch configuration after update")?;

        let stored_liquidity = updated_config
            .liquidity
            .context("Expected liquidity to be present after update")?;
        assert_eq!(
            stored_liquidity.status,
            FederationLaunchLiquidityStatus::FundsRequested
        );
        assert_eq!(
            stored_liquidity.gateway_liquidity,
            Amount::from_sats(100_000)
        );
        assert_eq!(
            stored_liquidity.stability_provision,
            Amount::from_sats(50_000)
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_update_liquidity_with_funded_status() -> Result<()> {
        use crate::amount::Amount;
        use crate::launch::configuration::{
            FederationLaunchLiquidity, FederationLaunchLiquidityStatus,
        };

        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        // First update with FundsRequested status
        let liquidity_requested = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::FundsRequested,
            gateway_liquidity: Amount::from_sats(100_000),
            stability_provision: Amount::from_sats(50_000),
        };
        db.update_liquidity(launch_config.launch_id.clone(), liquidity_requested)
            .await?;

        // Update to Funded status
        let liquidity_funded = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::Funded,
            gateway_liquidity: Amount::from_sats(150_000),
            stability_provision: Amount::from_sats(75_000),
        };
        db.update_liquidity(launch_config.launch_id.clone(), liquidity_funded.clone())
            .await?;

        // Verify the update
        let updated_config = db
            .get_federation_launch_configuration(&launch_config.launch_id)
            .await?
            .context("Failed to get federation launch configuration after update")?;

        let stored_liquidity = updated_config
            .liquidity
            .context("Expected liquidity to be present after update")?;
        assert_eq!(
            stored_liquidity.status,
            FederationLaunchLiquidityStatus::Funded
        );
        assert_eq!(
            stored_liquidity.gateway_liquidity,
            Amount::from_sats(150_000)
        );
        assert_eq!(
            stored_liquidity.stability_provision,
            Amount::from_sats(75_000)
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_update_liquidity_non_existent_launch_id() -> Result<()> {
        use crate::amount::Amount;
        use crate::launch::configuration::{
            FederationLaunchLiquidity, FederationLaunchLiquidityStatus,
        };

        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;

        let non_existent_id = FederationLaunchId::from_str("00000000-0000-0000-0000-000000000000")?;
        let liquidity = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::FundsRequested,
            gateway_liquidity: Amount::from_sats(100_000),
            stability_provision: Amount::from_sats(50_000),
        };

        let result = db.update_liquidity(non_existent_id, liquidity).await;
        assert!(matches!(
            result,
            Err(FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus)
        ));

        Ok(())
    }

    #[tokio::test]
    async fn test_search_launch_configurations_by_liquidity_status() -> Result<()> {
        use crate::amount::Amount;
        use crate::common::ChatUserId;
        use crate::launch::configuration::{
            FederationLaunchLiquidity, FederationLaunchLiquidityStatus,
        };

        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;

        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let plan = crate::launch::subscription::db::get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();
        // First subscribe to a plan
        let allow_internal_invoice = true;
        subscription_db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        standard_daemon::try_process_pending_subscription(&schema, &pool, &wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let fedimint_user_ids = vec![user_id.clone()];
        let og_user_ids = vec![user_id.clone()];

        // Create a federation with FundsRequested liquidity
        let launch_config_1 = db
            .create_standard_federation(
                user_id.clone(),
                "Test Federation 1".to_string(),
                1,
                1,
                1,
                fedimint_user_ids.clone(),
                og_user_ids.clone(),
            )
            .await?;

        let liquidity_funds_requested = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::FundsRequested,
            gateway_liquidity: Amount::from_sats(100_000),
            stability_provision: Amount::from_sats(50_000),
        };
        db.update_liquidity(launch_config_1.launch_id.clone(), liquidity_funds_requested)
            .await?;

        // Create second user with subscription for the second federation
        let user_id_2 = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        subscription_db
            .subscribe(&wallet, &user_id_2, plan.clone(), allow_internal_invoice)
            .await?;
        standard_daemon::try_process_pending_subscription(&schema, &pool, &wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let fedimint_user_ids_2 = vec![user_id_2.clone()];
        let og_user_ids_2 = vec![user_id_2.clone()];

        // Create another federation with Funded liquidity
        let launch_config_2 = db
            .create_standard_federation(
                user_id_2.clone(),
                "Test Federation 2".to_string(),
                1,
                1,
                1,
                fedimint_user_ids_2.clone(),
                og_user_ids_2.clone(),
            )
            .await?;

        let liquidity_funded = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::Funded,
            gateway_liquidity: Amount::from_sats(200_000),
            stability_provision: Amount::from_sats(100_000),
        };
        db.update_liquidity(launch_config_2.launch_id.clone(), liquidity_funded)
            .await?;

        // Create third user with subscription for the third federation
        let user_id_3 = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        subscription_db
            .subscribe(&wallet, &user_id_3, plan.clone(), allow_internal_invoice)
            .await?;
        standard_daemon::try_process_pending_subscription(&schema, &pool, &wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let fedimint_user_ids_3 = vec![user_id_3.clone()];
        let og_user_ids_3 = vec![user_id_3.clone()];

        // Create a third federation without liquidity data
        let _launch_config_3 = db
            .create_standard_federation(
                user_id_3,
                "Test Federation 3".to_string(),
                1,
                1,
                1,
                fedimint_user_ids_3.clone(),
                og_user_ids_3.clone(),
            )
            .await?;

        // Test filtering by FundsRequested status
        let results = db
            .search_launch_configurations(
                &fedimint_user_ids,
                &og_user_ids,
                &[],
                Some(&FederationLaunchLiquidityStatus::FundsRequested),
            )
            .await?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].launch_id, launch_config_1.launch_id);
        assert_eq!(
            results[0]
                .liquidity
                .as_ref()
                .context("Expected liquidity in results")?
                .status,
            FederationLaunchLiquidityStatus::FundsRequested
        );

        // Test filtering by Funded status
        let results = db
            .search_launch_configurations(
                &fedimint_user_ids_2,
                &og_user_ids_2,
                &[],
                Some(&FederationLaunchLiquidityStatus::Funded),
            )
            .await?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].launch_id, launch_config_2.launch_id);
        assert_eq!(
            results[0]
                .liquidity
                .as_ref()
                .context("Expected liquidity in results")?
                .status,
            FederationLaunchLiquidityStatus::Funded
        );

        // Test with just liquidity status filter (no user filters) to get all
        // federations with any liquidity
        let results_with_liquidity = db
            .search_launch_configurations(
                &[],
                &[],
                &[],
                Some(&FederationLaunchLiquidityStatus::FundsRequested),
            )
            .await?;
        // Should find the federation with FundsRequested status
        assert_eq!(results_with_liquidity.len(), 1);
        assert_eq!(
            results_with_liquidity[0].launch_id,
            launch_config_1.launch_id
        );

        let results_funded = db
            .search_launch_configurations(
                &[],
                &[],
                &[],
                Some(&FederationLaunchLiquidityStatus::Funded),
            )
            .await?;
        // Should find the federation with Funded status
        assert_eq!(results_funded.len(), 1);
        assert_eq!(results_funded[0].launch_id, launch_config_2.launch_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_search_launch_configurations_combined_filters_with_liquidity_status() -> Result<()>
    {
        use crate::amount::Amount;
        use crate::common::ChatUserId;
        use crate::launch::configuration::{
            FederationLaunchLiquidity, FederationLaunchLiquidityStatus, FederationLaunchStatus,
        };

        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;

        let user_id = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let plan = crate::launch::subscription::db::get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();
        // First subscribe to a plan
        let allow_internal_invoice = true;
        subscription_db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        standard_daemon::try_process_pending_subscription(&schema, &pool, &wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let fedimint_user_ids = vec![user_id.clone()];
        let og_user_ids = vec![user_id.clone()];

        // Create second user with subscription for the second federation
        let user_id_2 = ChatUserId::from(format!("TESTUSER-{id}", id = Uuid::new_v4()));
        subscription_db
            .subscribe(&wallet, &user_id_2, plan.clone(), allow_internal_invoice)
            .await?;
        standard_daemon::try_process_pending_subscription(&schema, &pool, &wallet)
            .await?
            .context("Failed to process pending subscription")?;

        let other_fedimint_user_ids = vec![user_id_2.clone()];

        // Create federation with FundsRequested and set to InProgress status
        let launch_config_1 = db
            .create_standard_federation(
                user_id.clone(),
                "Test Federation 1".to_string(),
                1,
                1,
                1,
                fedimint_user_ids.clone(),
                og_user_ids.clone(),
            )
            .await?;

        let liquidity_funds_requested = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::FundsRequested,
            gateway_liquidity: Amount::from_sats(100_000),
            stability_provision: Amount::from_sats(50_000),
        };
        db.update_liquidity(launch_config_1.launch_id.clone(), liquidity_funds_requested)
            .await?;
        db.update_status(
            launch_config_1.launch_id.clone(),
            FederationLaunchStatus::InProgress,
        )
        .await?;

        // Create another federation with FundsRequested but different users and status
        let launch_config_2 = db
            .create_standard_federation(
                user_id_2.clone(),
                "Test Federation 2".to_string(),
                1,
                1,
                1,
                other_fedimint_user_ids.clone(),
                vec![user_id_2.clone()],
            )
            .await?;

        let liquidity_funds_requested = FederationLaunchLiquidity {
            status: FederationLaunchLiquidityStatus::FundsRequested,
            gateway_liquidity: Amount::from_sats(200_000),
            stability_provision: Amount::from_sats(100_000),
        };
        db.update_liquidity(launch_config_2.launch_id.clone(), liquidity_funds_requested)
            .await?;
        // Keep this one in Requested status

        // Test combined filters: specific fedimint users + FundsRequested liquidity +
        // InProgress status
        let results = db
            .search_launch_configurations(
                &fedimint_user_ids,
                &[],
                &[FederationLaunchStatus::InProgress],
                Some(&FederationLaunchLiquidityStatus::FundsRequested),
            )
            .await?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].launch_id, launch_config_1.launch_id);
        assert_eq!(results[0].status, FederationLaunchStatus::InProgress);
        assert_eq!(
            results[0]
                .liquidity
                .as_ref()
                .context("Expected liquidity in results")?
                .status,
            FederationLaunchLiquidityStatus::FundsRequested
        );

        // Test combined filters that should return no results
        let results = db
            .search_launch_configurations(
                &fedimint_user_ids,
                &[],
                &[FederationLaunchStatus::Ready], // No federation has this status
                Some(&FederationLaunchLiquidityStatus::FundsRequested),
            )
            .await?;
        assert_eq!(results.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_search_launch_configurations_empty_filters_with_liquidity_status() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;

        // Test that when all filter arrays are empty but liquidity_status is provided,
        // we should return results (the early return logic should not trigger)
        let results = db
            .search_launch_configurations(
                &[], // empty fedimint_user_ids
                &[], // empty og_user_ids
                &[], // empty status
                Some(&FederationLaunchLiquidityStatus::FundsRequested),
            )
            .await?;

        // Should return empty results but not early return
        assert_eq!(results.len(), 0);

        // Test that when all filters are empty including liquidity_status,
        // we should get early return with empty results
        let results = db
            .search_launch_configurations(
                &[],  // empty fedimint_user_ids
                &[],  // empty og_user_ids
                &[],  // empty status
                None, // no liquidity_status
            )
            .await?;

        assert_eq!(results.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_set_invite_code_success() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;

        let wallet = mock_random_invoice(FelaasWallet::faux());
        let launch_config =
            create_subscription_test_launch(&schema, &pool, &wallet, &db, &subscription_db).await?;

        // Test setting invite code with database operation
        let invite_code = InviteCode::from_str(
            "fed11qgqpw9thwvaz7te3xgmjuvpwxqhrzw33xvurgwf0qqqjqzp0as69398jqst9r5jcf04uqgea5pdlmuhxlqw5zhst2fmqlqkvs387m3",
        )?;
        db.set_invite_code(launch_config.launch_id.clone(), invite_code.clone())
            .await?;

        // Verify at database level that the invite code was stored
        let client = db.pool.get().await?;
        let schema = &db.schema;
        let query = format!(
            "SELECT invite_code FROM {schema}.{FEDERATION_LAUNCH_CONFIGURATION} WHERE launch_id = $1"
        );
        let row = client
            .query_one(&query, &[&launch_config.launch_id])
            .await?;
        let stored_invite_code: Option<InviteCodeSql> = row.get("invite_code");
        assert_eq!(stored_invite_code, Some(InviteCodeSql::from(invite_code)));

        Ok(())
    }

    #[tokio::test]
    async fn test_set_invite_code_non_existent_launch_id() -> Result<()> {
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let pool = test_db_pool().await?;
        let db = FederationLauncherDB::new(pool.clone(), schema.clone());
        db.create_table_if_not_exists().await?;

        let non_existent_id = FederationLaunchId::from_str("00000000-0000-0000-0000-000000000000")?;
        let invite_code = InviteCode::from_str(
            "fed11qgqpw9thwvaz7te3xgmjuvpwxqhrzw33xvurgwf0qqqjqzp0as69398jqst9r5jcf04uqgea5pdlmuhxlqw5zhst2fmqlqkvs387m3",
        )?;
        let result = db.set_invite_code(non_existent_id, invite_code).await;
        assert!(matches!(
            result,
            Err(FederationLaunchError::LaunchIdNotFoundOrIsInInvalidStatus)
        ));

        Ok(())
    }
}
