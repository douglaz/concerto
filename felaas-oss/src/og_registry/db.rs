use chrono::{DateTime, Utc};
use postgres_from_row::FromRow;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::common::ChatUserId;
use crate::og_registry::error::{OgRegistryError, Result};
use crate::PgPool;

const OG_REGISTRY_TABLE: &str = "og_registry";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OgRecord {
    pub user_id: ChatUserId,
    pub reference_user_id: ChatUserId,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct OgRegistryDB {
    pool: PgPool,
    schema: String,
}

impl OgRegistryDB {
    pub fn new(pool: PgPool, schema: String) -> Self {
        Self { pool, schema }
    }

    pub async fn create_table_if_not_exists(&self) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let schema = &self.schema;

        // Create schema if not exists
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

        // Create table with indexes
        client
            .batch_execute(&format!(
                r#"
                CREATE TABLE IF NOT EXISTS {schema}.{OG_REGISTRY_TABLE} (
                    user_id TEXT PRIMARY KEY,
                    reference_user_id TEXT NOT NULL,
                    is_active BOOLEAN NOT NULL DEFAULT true,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    UNIQUE (user_id, reference_user_id)
                );
                
                CREATE INDEX IF NOT EXISTS idx_{schema}_{OG_REGISTRY_TABLE}_is_active
                ON {schema}.{OG_REGISTRY_TABLE} (is_active) WHERE is_active = true;
                
                CREATE INDEX IF NOT EXISTS idx_{schema}_{OG_REGISTRY_TABLE}_updated_at
                ON {schema}.{OG_REGISTRY_TABLE} (updated_at);
                
                -- Ensure only one active OG per reference_user_id
                CREATE UNIQUE INDEX IF NOT EXISTS idx_{schema}_{OG_REGISTRY_TABLE}_reference_user_id_active
                ON {schema}.{OG_REGISTRY_TABLE} (reference_user_id) WHERE is_active = true;
                "#
            ))
            .await?;

        info!("OG registry table created/verified");
        Ok(())
    }

    /// Upsert an OG (insert or update)
    pub async fn upsert_og(
        &self,
        user_id: ChatUserId,
        reference_user_id: ChatUserId,
        is_active: bool,
    ) -> Result<OgRecord> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        let schema = &self.schema;

        // If activating, check if another OG with same reference_user_id is already
        // active
        if is_active {
            let existing = tx
                .query_opt(
                    &format!(
                        r#"
                        SELECT user_id FROM {schema}.{OG_REGISTRY_TABLE}
                        WHERE reference_user_id = $1 AND is_active = true AND user_id != $2
                        FOR UPDATE
                        "#
                    ),
                    &[&reference_user_id, &user_id],
                )
                .await?;

            if let Some(row) = existing {
                let existing_user_id: ChatUserId = row.get(0);
                return Err(OgRegistryError::DuplicateActiveOg {
                    reference_user_id,
                    existing_user_id,
                });
            }
        }

        let row = tx
            .query_one(
                &format!(
                    r#"
                    INSERT INTO {schema}.{OG_REGISTRY_TABLE} (user_id, reference_user_id, is_active, updated_at)
                    VALUES ($1, $2, $3, NOW())
                    ON CONFLICT (user_id, reference_user_id) 
                    DO UPDATE SET 
                        is_active = $3,
                        updated_at = NOW()
                    RETURNING user_id, reference_user_id, is_active, created_at, updated_at
                    "#
                ),
                &[&user_id, &reference_user_id, &is_active],
            )
            .await?;

        let og = OgRecord::try_from_row(&row)
            .map_err(|e| OgRegistryError::RowConversionError(e.to_string()))?;

        tx.commit().await?;
        debug!(?og, "OG upserted successfully");
        Ok(og)
    }

    /// Get a specific OG by user_id
    pub async fn get_og(&self, user_id: &ChatUserId) -> Result<Option<OgRecord>> {
        let client = self.pool.get().await?;
        let schema = &self.schema;

        let result = client
            .query_opt(
                &format!(
                    r#"
                    SELECT user_id, reference_user_id, is_active, created_at, updated_at
                    FROM {schema}.{OG_REGISTRY_TABLE}
                    WHERE user_id = $1
                    "#
                ),
                &[&user_id],
            )
            .await?;

        match result {
            Some(row) => {
                let og = OgRecord::try_from_row(&row)
                    .map_err(|e| OgRegistryError::RowConversionError(e.to_string()))?;
                Ok(Some(og))
            }
            None => Ok(None),
        }
    }

    /// List all active OGs
    pub async fn list_active_ogs(&self) -> Result<Vec<OgRecord>> {
        let client = self.pool.get().await?;
        let schema = &self.schema;

        let rows = client
            .query(
                &format!(
                    r#"
                    SELECT user_id, reference_user_id, is_active, created_at, updated_at
                    FROM {schema}.{OG_REGISTRY_TABLE}
                    WHERE is_active = true
                    ORDER BY updated_at DESC
                    "#
                ),
                &[],
            )
            .await?;

        let ogs = rows
            .iter()
            .map(|row| {
                OgRecord::try_from_row(row)
                    .map_err(|e| OgRegistryError::RowConversionError(e.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;

        debug!("Listed {count} active OGs", count = ogs.len());
        Ok(ogs)
    }

    /// Get N random active OGs for federation assignment
    pub async fn get_random_active_ogs(&self, count: i64) -> Result<Vec<ChatUserId>> {
        let client = self.pool.get().await?;
        let schema = &self.schema;

        let rows = client
            .query(
                &format!(
                    r#"
                    SELECT user_id
                    FROM {schema}.{OG_REGISTRY_TABLE}
                    WHERE is_active = true
                    ORDER BY RANDOM()
                    LIMIT $1
                    "#
                ),
                &[&count],
            )
            .await?;

        let og_ids: Vec<ChatUserId> = rows
            .iter()
            .map(|row| {
                let user_id: ChatUserId = row.get(0);
                user_id
            })
            .collect();

        debug!("Selected {count} random active OGs", count = og_ids.len());
        Ok(og_ids)
    }
}
