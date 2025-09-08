//! Subscription plans and user subscription database access.
//!
//! This module defines types and database operations for managing subscription
//! plans and user subscriptions.

use anyhow::{Context, Result};
use postgres_from_row::FromRow;
use thiserror::Error;
use tracing::debug;

use crate::PgPool;
use crate::amount::Amount;
use crate::common::ChatUserId;
use crate::launch::subscription::{
    PlanType, Subscription, SubscriptionCancellationReason, SubscriptionPayment,
    SubscriptionPaymentError, SubscriptionPaymentId, SubscriptionPaymentInvoiceDetails,
    SubscriptionPaymentStatus, SubscriptionStatus,
};
use crate::wallet::core::FelaasWallet;

const LAST_UPDATED_AT_COLUMN: &str = "last_updated_at";
const CREATED_AT_COLUMN: &str = "created_at";

// Table and column names for the subscriptions and plan catalog tables.
const SUBSCRIPTIONS_TABLE: &str = "user_subscription";
const USER_ID_COLUMN: &str = "user_id";
const PLAN_COLUMN: &str = "plan";
const STATUS_COLUMN: &str = "status";
const CANCELLED_REASON_COLUMN: &str = "cancelled_reason";
const ACTIVATED_AT_COLUMN: &str = "activated_at";

// Table and column names for subscription payments
const SUBSCRIPTION_PAYMENTS_TABLE: &str = "subscription_payment";
const ID_COLUMN: &str = "id";
const AMOUNT_COLUMN: &str = "amount";
const PAYMENT_REQUEST_COLUMN: &str = "payment_request";
const PREIMAGE_COLUMN: &str = "preimage";
const FAILURE_REASON_COLUMN: &str = "failure_reason";

/// Error type for subscription operations.
#[derive(Debug, Error)]
pub enum SubscriptionError {
    #[error("Subscription already exists for user: {0}")]
    AlreadyExists(ChatUserId),

    #[error("Plan not found: {0:?}")]
    PlanNotFound(PlanType),

    #[error("Database error: {0}")]
    DatabaseError(#[from] tokio_postgres::Error),

    #[error("Database pool error: {0}")]
    DatabasePoolError(#[from] deadpool_postgres::PoolError),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

#[derive(Clone)]
pub struct SubscriptionDB {
    pool: PgPool,
    schema: String,
}

impl SubscriptionDB {
    pub fn new(pool: PgPool, schema: String) -> Self {
        Self { pool, schema }
    }

    /// Creates the plan catalog and subscriptions tables if they do not exist.
    pub async fn create_table_if_not_exists(&self) -> Result<()> {
        let client = self
            .pool
            .get()
            .await
            .context("Failed to get DB connection for creating tables")?;
        let schema = &self.schema;

        let create_table_sql = format!(
            r#"
            CREATE SCHEMA IF NOT EXISTS {schema};
            CREATE TABLE IF NOT EXISTS {schema}.{SUBSCRIPTIONS_TABLE} (
                {USER_ID_COLUMN} VARCHAR(255) NOT NULL,
                {PLAN_COLUMN} JSONB NOT NULL,
                {CANCELLED_REASON_COLUMN} JSONB,
                {CREATED_AT_COLUMN} TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                {LAST_UPDATED_AT_COLUMN} TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                {ACTIVATED_AT_COLUMN} TIMESTAMPTZ,
                {STATUS_COLUMN} VARCHAR(255) NOT NULL,
                PRIMARY KEY ({USER_ID_COLUMN})
            );

            CREATE INDEX IF NOT EXISTS idx_{schema}_subscription_status ON {schema}.{SUBSCRIPTIONS_TABLE}({STATUS_COLUMN});
            CREATE INDEX IF NOT EXISTS idx_{schema}_subscription_created_at ON {schema}.{SUBSCRIPTIONS_TABLE}({CREATED_AT_COLUMN});

            CREATE TABLE IF NOT EXISTS {schema}.{SUBSCRIPTION_PAYMENTS_TABLE} (
                {ID_COLUMN} UUID PRIMARY KEY,
                {USER_ID_COLUMN} VARCHAR(255) NOT NULL,
                {PLAN_COLUMN} JSONB NOT NULL,
                {AMOUNT_COLUMN} BIGINT NOT NULL,
                {PAYMENT_REQUEST_COLUMN} JSONB NOT NULL,
                {PREIMAGE_COLUMN} TEXT,
                {FAILURE_REASON_COLUMN} JSONB,
                {STATUS_COLUMN} VARCHAR(255) NOT NULL,
                {CREATED_AT_COLUMN} TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                {LAST_UPDATED_AT_COLUMN} TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );

            CREATE INDEX IF NOT EXISTS idx_{schema}_subscription_payment_status ON {schema}.{SUBSCRIPTION_PAYMENTS_TABLE}({STATUS_COLUMN});
            CREATE INDEX IF NOT EXISTS idx_{schema}_subscription_payment_created_at ON {schema}.{SUBSCRIPTION_PAYMENTS_TABLE}({CREATED_AT_COLUMN});
        "#,
        );
        // no transaction needed as it is a single table
        client
            .batch_execute(&create_table_sql)
            .await
            .context("Failed to execute batch for creating tables")?;
        Ok(())
    }

    /// Registers or updates a user's subscription to a plan in the database.
    pub async fn subscribe(
        &self,
        wallet: &FelaasWallet,
        user_id: &ChatUserId,
        plan: PlanType,
        allow_internal_invoice: bool,
    ) -> Result<(Subscription, SubscriptionPayment), SubscriptionError> {
        let schema = &self.schema;
        let mut client = self
            .pool
            .get()
            .await
            .context("Failed to get DB connection for subscribe")?;
        let tx = client.transaction().await?;
        let plan_name = plan.name();
        let subscription = subscribe(schema, &tx, user_id, plan).await?;
        let memo = serde_json::to_string(&SubscriptionPaymentInvoiceDetails {
            user_id: user_id.clone(),
            plan_name,
        })
        .map_err(anyhow::Error::new)?;
        let crate::wallet::core::InvoiceCreatedDetails {
            operation_id,
            invoice,
        } = wallet
            .create_internal_wallet_invoice(
                subscription.plan.price(),
                Some(60 * 60 * 24 * 7), // 1 week
                fedimint_ln_common::lightning_invoice::Description::new(memo)
                    .context("Failed to create description")?,
                allow_internal_invoice,
            )
            .await
            .context("Failed to create invoice")?;
        let subscription_payment =
            crate::launch::subscription::db::create_pending_subscription_payment(
                schema,
                &tx,
                &subscription,
                super::PaymentRequest {
                    invoice,
                    operation_id,
                },
            )
            .await
            .context("Failed to create subscription payment")?;
        tx.commit().await?;
        debug!(?subscription, ?subscription_payment, "Created subscription");
        Ok((subscription, subscription_payment))
    }

    pub async fn get_current_subscription(
        &self,
        user_id: &ChatUserId,
    ) -> Result<Option<Subscription>, SubscriptionError> {
        let mut pool = self.pool.get().await?;
        let tx = pool.transaction().await?;
        get_current_subscription(&self.schema, &tx, user_id).await
    }

    #[cfg(test)]
    pub async fn find_latest_subscription_payment_by_user(
        &self,
        user_id: &ChatUserId,
    ) -> Result<Option<SubscriptionPayment>> {
        let mut pool = self.pool.get().await?;
        let tx = pool.transaction().await?;
        find_latest_subscription_payment_by_user(&self.schema, &tx, user_id).await
    }
}

pub async fn subscribe(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    user_id: &ChatUserId,
    plan: PlanType,
) -> Result<Subscription, SubscriptionError> {
    // Check if plan exists
    let plan_catalog = get_plan_catalog();
    let plan = plan_catalog
        .iter()
        .find(|p| **p == plan)
        .ok_or(SubscriptionError::PlanNotFound(plan))?;

    // Attempt to insert new subscription with current timestamp
    let statement = tx.prepare_cached(&format!(
        r#"
            INSERT INTO {schema}.{SUBSCRIPTIONS_TABLE} ({USER_ID_COLUMN}, {PLAN_COLUMN}, {CREATED_AT_COLUMN}, {STATUS_COLUMN})
            VALUES ($1, $2, NOW(), $3)
            RETURNING *
        "#,
    )).await?;
    let result = tx
        .query_one(
            &statement,
            &[
                user_id,
                &plan,
                &SubscriptionStatus::PendingInitialActivation,
            ],
        )
        .await;

    match result {
        Ok(row) => Ok(Subscription::try_from_row(&row).map_err(anyhow::Error::new)?),
        Err(e) => {
            // Check for unique violation (user already subscribed issue caused by
            // concurrency)
            let db_err: &tokio_postgres::error::DbError = e
                .as_db_error()
                .context("Failed to insert new user subscription")?;
            if db_err.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION {
                Err(SubscriptionError::AlreadyExists(user_id.to_owned()))
            } else {
                Err(anyhow::Error::new(e)
                    .context("Failed to insert new user subscription")
                    .into())
            }
        }
    }
}

pub async fn update_subscription_status(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    user_id: &ChatUserId,
    previous_status: SubscriptionStatus,
    status: SubscriptionStatus,
    cancelled_reason: Option<SubscriptionCancellationReason>,
) -> Result<Subscription> {
    let extra = if status == SubscriptionStatus::Active {
        format!(", {ACTIVATED_AT_COLUMN} = NOW()")
    } else {
        String::new()
    };
    let query = format!(
        r#"
            UPDATE {schema}.{SUBSCRIPTIONS_TABLE}
            SET {STATUS_COLUMN} = $1, {CANCELLED_REASON_COLUMN} = $2, {LAST_UPDATED_AT_COLUMN} = NOW()
            {extra}
            WHERE {USER_ID_COLUMN} = $3 AND {STATUS_COLUMN} = $4
            RETURNING *
        "#
    );

    let statement = tx.prepare_cached(&query).await?;
    let row = tx
        .query_one(
            &statement,
            &[&status, &cancelled_reason, &user_id, &previous_status],
        )
        .await
        .context("Failed to update subscription status")?;

    Subscription::try_from_row(&row).map_err(anyhow::Error::new)
}

pub async fn find_latest_subscription_payment_by_user(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    user_id: &ChatUserId,
) -> Result<Option<SubscriptionPayment>> {
    let query = format!(
        r#"
            SELECT *
            FROM {schema}.{SUBSCRIPTION_PAYMENTS_TABLE}
            WHERE {USER_ID_COLUMN} = $1
            ORDER BY {CREATED_AT_COLUMN} DESC
            LIMIT 1
            FOR UPDATE
        "#
    );
    let statement = tx.prepare_cached(&query).await?;

    let row = tx
        .query_opt(&statement, &[user_id])
        .await
        .context("Failed to query subscription payments")?;

    let payment = row
        .map(|row| SubscriptionPayment::try_from_row(&row).map_err(anyhow::Error::new))
        .transpose()
        .context("Failed to parse subscription payment")?;

    Ok(payment)
}

/// Get the oldest pending subscription
pub async fn get_oldest_pending_subscription(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
) -> Result<Option<Subscription>> {
    let query = format!(
        r#"
            SELECT *
            FROM {schema}.{SUBSCRIPTIONS_TABLE}
            WHERE {STATUS_COLUMN} = $1
            ORDER BY {CREATED_AT_COLUMN} ASC
            LIMIT 1
            FOR UPDATE
        "#
    );
    let statement = tx.prepare_cached(&query).await?;

    let row = tx
        .query_opt(&statement, &[&SubscriptionStatus::PendingInitialActivation])
        .await
        .context("Failed to query subscriptions")?;

    let subscription = row
        .map(|row| Subscription::try_from_row(&row).map_err(anyhow::Error::new))
        .transpose()
        .context("Failed to parse subscription")?;

    Ok(subscription)
}

pub async fn create_pending_subscription_payment(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    subscription: &Subscription,
    payment_request: super::PaymentRequest,
) -> Result<SubscriptionPayment> {
    let query = format!(
        r#"
            INSERT INTO {schema}.{SUBSCRIPTION_PAYMENTS_TABLE}
            ({ID_COLUMN}, {USER_ID_COLUMN}, {PLAN_COLUMN}, {AMOUNT_COLUMN}, {PAYMENT_REQUEST_COLUMN}, {STATUS_COLUMN})
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *;
        "#
    );
    let statement = tx.prepare_cached(&query).await?;
    let row = tx
        .query_one(
            &statement,
            &[
                &SubscriptionPaymentId(uuid::Uuid::new_v4()),
                &subscription.user_id,
                &subscription.plan,
                &subscription.plan.price(),
                &payment_request,
                &SubscriptionPaymentStatus::Pending,
            ],
        )
        .await
        .context("Failed to insert subscription payment")?;
    let subscription_payment =
        SubscriptionPayment::try_from_row(&row).map_err(anyhow::Error::new)?;
    Ok(subscription_payment)
}

pub async fn update_subscription_payment(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    payment_id: &SubscriptionPaymentId,
    preimage: Option<&String>,
    failure_reason: Option<SubscriptionPaymentError>,
    previous_status: SubscriptionPaymentStatus,
    status: SubscriptionPaymentStatus,
) -> Result<SubscriptionPayment> {
    let query = format!(
        r#"
            UPDATE {schema}.{SUBSCRIPTION_PAYMENTS_TABLE}
            SET {PREIMAGE_COLUMN} = $1, {FAILURE_REASON_COLUMN} = $2, {STATUS_COLUMN} = $3, {LAST_UPDATED_AT_COLUMN} = NOW()
            WHERE {ID_COLUMN} = $4 AND {STATUS_COLUMN} = $5
            RETURNING *
        "#
    );

    let statement = tx.prepare_cached(&query).await?;

    let row = tx
        .query_one(
            &statement,
            &[
                &preimage,
                &failure_reason,
                &status,
                &payment_id,
                &previous_status,
            ],
        )
        .await
        .context("Failed to update subscription payment")?;

    SubscriptionPayment::try_from_row(&row).map_err(anyhow::Error::new)
}

/// Returns the current subscription for a user from the database, if any.
pub async fn get_current_subscription(
    schema: &str,
    tx: &deadpool_postgres::Transaction<'_>,
    user_id: &ChatUserId,
) -> Result<Option<Subscription>, SubscriptionError> {
    let query = format!(
        "SELECT * FROM {schema}.{SUBSCRIPTIONS_TABLE} WHERE {USER_ID_COLUMN} = $1 FOR UPDATE"
    );

    let statement = tx.prepare_cached(&query).await?;
    let rows = tx
        .query(&statement, &[user_id])
        .await
        .context("Failed to query current subscription for user")?;

    if rows.len() > 1 {
        return Err(anyhow::anyhow!("Multiple subscriptions found for user").into());
    }

    if let Some(row) = rows.first() {
        Ok(Some(Subscription::try_from_row(row)?))
    } else {
        Ok(None)
    }
}

pub fn _old_reference_plan_catalog() -> Vec<PlanType> {
    vec![
        PlanType::V0 {
            name: "FriendsWithBenefits".into(),
            num_fedimints: 1,
            num_ogs: 3,
            price: Amount::from_sats(1),
            renew_months: 1,
        },
        PlanType::V0 {
            name: "OneFMThreeOG".into(),
            num_fedimints: 1,
            num_ogs: 3,
            price: Amount::from_sats(40),
            renew_months: 3,
        },
        PlanType::V0 {
            name: "FullOG".into(),
            num_fedimints: 4,
            num_ogs: 0,
            price: Amount::from_sats(990),
            renew_months: 3,
        },
        PlanType::V0 {
            name: "SixMonthsPlan".into(),
            num_fedimints: 1,
            num_ogs: 3,
            price: Amount::from_sats(120_000),
            renew_months: 6,
        },
        PlanType::V0 {
            name: "TwelveMonthsPlan".into(),
            num_fedimints: 1,
            num_ogs: 3,
            price: Amount::from_sats(240_000),
            renew_months: 12,
        },
    ]
}

/// Returns all available plans from the plan catalog.
pub fn get_plan_catalog() -> Vec<PlanType> {
    vec![PlanType::V0 {
        name: "FriendsWithoutBenefits".into(),
        num_fedimints: 1,
        num_ogs: 3,
        price: Amount::from_bitcoins(1),
        renew_months: 1,
    }]
}

#[cfg(test)]
mod tests {
    use std::env;

    use anyhow::Result;
    use deadpool_postgres::{Config, Pool, Runtime};
    use tokio_postgres::NoTls;

    use super::*;
    use crate::amount::Amount;
    use crate::common::{ChatUserId, mock_random_invoice};
    use crate::{PGDATABASE, PGHOST, PGPASSWORD, PGPORT, PGUSER};

    // Helper to create a test database pool
    async fn test_db_pool() -> Result<Pool> {
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
        Ok(cfg.create_pool(Some(Runtime::Tokio1), NoTls)?)
    }

    #[tokio::test]
    async fn test_subscription_serialization() -> Result<()> {
        let sub = Subscription {
            user_id: ChatUserId::from("user123"),
            plan: PlanType::V0 {
                name: "asdjal".to_owned(),
                num_fedimints: 3,
                num_ogs: 1,
                price: Amount::from_sats(100),
                renew_months: 3,
            },
            status: SubscriptionStatus::PendingInitialActivation,
            created_at: chrono::Utc::now(),
            activated_at: None,
            last_updated_at: chrono::Utc::now(),
            cancelled_reason: None,
        };
        let json = serde_json::to_string(&sub)?;
        let deserialized: Subscription = serde_json::from_str(&json)?;
        assert_eq!(sub, deserialized);
        Ok(())
    }

    #[tokio::test]
    async fn test_subscription_db_lifecycle() -> Result<()> {
        let pool = test_db_pool().await?;
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let db = SubscriptionDB {
            pool: pool.clone(),
            schema: schema.clone(),
        };
        db.create_table_if_not_exists().await?;
        let wallet = mock_random_invoice(FelaasWallet::faux());
        let user_id = ChatUserId::from("testuser1");
        let plan = get_plan_catalog()
            .first()
            .context("Failed to get plan")?
            .to_owned();

        // Subscribe
        let allow_internal_invoice = true;
        let (subscription, payment) = db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await?;
        assert_eq!(subscription.user_id, user_id);
        assert_eq!(subscription.plan, plan);
        assert_eq!(payment.status, SubscriptionPaymentStatus::Pending);

        // Duplicate subscribe should error
        let subscribe_result = db
            .subscribe(&wallet, &user_id, plan.clone(), allow_internal_invoice)
            .await;
        match subscribe_result {
            Err(SubscriptionError::AlreadyExists(uid)) => assert_eq!(uid, user_id),
            _ => unreachable!("Should have returned already exists error"),
        }

        // Get current subscription
        let current = db.get_current_subscription(&user_id).await?;
        assert!(current.is_some());
        if let Some(current) = current {
            assert_eq!(current.user_id, user_id);
            assert_eq!(current.plan, plan);
            assert_eq!(current.status, SubscriptionStatus::PendingInitialActivation);
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_get_plan_catalog() -> Result<()> {
        let plans = crate::launch::subscription::db::get_plan_catalog();
        assert!(!plans.is_empty());
        for plan in plans {
            let (name, num_fedimints, _num_ogs, price, renew_months) = match plan {
                PlanType::V0 {
                    name,
                    num_fedimints,
                    num_ogs,
                    price,
                    renew_months,
                } => (name, num_fedimints, num_ogs, price, renew_months),
            };
            assert!(!name.is_empty());
            assert!(num_fedimints > 0);
            assert!(price > Amount::from_sats(0));
            assert!(renew_months > 0);
        }
        Ok(())
    }
}
