use chrono::{DateTime, Utc};
use concerto_common::*;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct HostedSlot {
    pub slot_id: Uuid,
    pub guardian_npub: String,
    pub federation_id: String,
    pub subscription_id: Uuid,
    pub deployment_id: Option<String>,
    pub service_endpoint: Option<String>,
    pub status: String,
    pub allocated_at: DateTime<Utc>,
    pub last_health_check: Option<DateTime<Utc>>,
    pub cpu_hours: f32,
    pub bandwidth_gb: f32,
    pub storage_gb: f32,
    pub request_count: i64,
    pub pricing_tier: String,
    pub base_price_sats: i64,
    pub usage_fees_sats: i64,
    pub total_billed_sats: i64,
    pub last_invoice_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct PricingInfo {
    pub base_slot_price_sats: u64,
    pub cpu_per_core_hour_sats: u64,
    pub memory_per_gb_hour_sats: u64,
    pub storage_per_gb_month_sats: u64,
    pub bandwidth_per_gb_sats: u64,
}

#[derive(Debug, Clone)]
pub struct RevenueReport {
    pub total_revenue_sats: u64,
    pub total_costs_sats: u64,
    pub profit_sats: i64,
    pub profit_margin_percent: f32,
}

#[derive(Debug, Clone)]
pub struct UtilizationReport {
    pub total_slots: u32,
    pub active_slots: u32,
    pub utilization_percent: f32,
    pub avg_cpu_percent: f32,
    pub avg_memory_percent: f32,
}

pub async fn init_database(pool: &PgPool) -> anyhow::Result<()> {
    // Create tables
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS subscription_validations (
            subscription_id UUID PRIMARY KEY,
            owner_npub TEXT NOT NULL,
            tier TEXT NOT NULL,
            payment_status TEXT NOT NULL,
            last_payment_date TIMESTAMP,
            credit_score INTEGER,
            valid_until TIMESTAMP NOT NULL,
            total_slots_allocated INTEGER DEFAULT 0,
            total_revenue_sats BIGINT DEFAULT 0,
            payment_history JSONB,
            CONSTRAINT valid_credit_score CHECK (credit_score BETWEEN 0 AND 1000)
        );
        
        CREATE TABLE IF NOT EXISTS hosted_slots (
            slot_id UUID PRIMARY KEY,
            guardian_npub TEXT NOT NULL,
            federation_id TEXT NOT NULL,
            subscription_id UUID NOT NULL,
            deployment_id TEXT,
            service_endpoint TEXT,
            status TEXT NOT NULL,
            allocated_at TIMESTAMP NOT NULL,
            last_health_check TIMESTAMP,
            cpu_hours DECIMAL DEFAULT 0,
            bandwidth_gb DECIMAL DEFAULT 0,
            storage_gb DECIMAL DEFAULT 0,
            request_count BIGINT DEFAULT 0,
            pricing_tier TEXT NOT NULL,
            base_price_sats BIGINT NOT NULL,
            usage_fees_sats BIGINT DEFAULT 0,
            total_billed_sats BIGINT DEFAULT 0,
            last_invoice_date TIMESTAMP,
            INDEX idx_subscription (subscription_id),
            INDEX idx_federation (federation_id),
            INDEX idx_status (status)
        );
        
        CREATE TABLE IF NOT EXISTS provider_economics (
            month DATE PRIMARY KEY,
            total_slots_hosted INTEGER,
            total_revenue_sats BIGINT,
            total_costs_sats BIGINT,
            profit_margin_percent DECIMAL,
            average_utilization_percent DECIMAL,
            peak_demand_multiplier DECIMAL,
            tier_distribution JSONB,
            payment_method_distribution JSONB
        );
        
        CREATE TABLE IF NOT EXISTS pricing_history (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            effective_date TIMESTAMP NOT NULL,
            base_slot_price_sats BIGINT NOT NULL,
            resource_prices JSONB NOT NULL,
            demand_curve JSONB,
            competitor_average_price BIGINT,
            market_demand_index DECIMAL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        
        CREATE TABLE IF NOT EXISTS invoices (
            invoice_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            subscription_id UUID NOT NULL,
            slot_id UUID,
            billing_period_start DATE NOT NULL,
            billing_period_end DATE NOT NULL,
            base_fee_sats BIGINT NOT NULL,
            usage_fees_sats BIGINT NOT NULL,
            total_sats BIGINT NOT NULL,
            payment_status TEXT NOT NULL DEFAULT 'pending',
            payment_method TEXT,
            paid_at TIMESTAMP,
            payment_proof TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            due_date TIMESTAMP NOT NULL,
            INDEX idx_subscription_invoices (subscription_id),
            INDEX idx_payment_status (payment_status)
        );
        "#,
    )
    .execute(pool)
    .await?;
    
    // Insert default pricing if none exists
    sqlx::query(
        r#"
        INSERT INTO pricing_history (
            effective_date,
            base_slot_price_sats,
            resource_prices
        )
        SELECT NOW(), 100000, '{
            "cpu_per_core_hour_sats": 100,
            "memory_per_gb_hour_sats": 50,
            "storage_per_gb_month_sats": 1000,
            "bandwidth_per_gb_sats": 10
        }'::jsonb
        WHERE NOT EXISTS (SELECT 1 FROM pricing_history)
        "#,
    )
    .execute(pool)
    .await?;
    
    Ok(())
}

pub async fn insert_slot(pool: &PgPool, slot: &HostedSlot) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hosted_slots (
            slot_id, guardian_npub, federation_id, subscription_id,
            deployment_id, service_endpoint, status, allocated_at,
            last_health_check, cpu_hours, bandwidth_gb, storage_gb,
            request_count, pricing_tier, base_price_sats, usage_fees_sats,
            total_billed_sats, last_invoice_date
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        "#,
    )
    .bind(slot.slot_id)
    .bind(&slot.guardian_npub)
    .bind(&slot.federation_id)
    .bind(slot.subscription_id)
    .bind(&slot.deployment_id)
    .bind(&slot.service_endpoint)
    .bind(&slot.status)
    .bind(slot.allocated_at)
    .bind(slot.last_health_check)
    .bind(slot.cpu_hours)
    .bind(slot.bandwidth_gb)
    .bind(slot.storage_gb)
    .bind(slot.request_count)
    .bind(&slot.pricing_tier)
    .bind(slot.base_price_sats)
    .bind(slot.usage_fees_sats)
    .bind(slot.total_billed_sats)
    .bind(slot.last_invoice_date)
    .execute(pool)
    .await?;
    
    Ok(())
}

pub async fn list_slots(pool: &PgPool) -> anyhow::Result<Vec<HostedSlot>> {
    let rows = sqlx::query(
        "SELECT * FROM hosted_slots ORDER BY allocated_at DESC"
    )
    .fetch_all(pool)
    .await?;
    
    let slots = rows
        .into_iter()
        .map(|row| HostedSlot {
            slot_id: row.get("slot_id"),
            guardian_npub: row.get("guardian_npub"),
            federation_id: row.get("federation_id"),
            subscription_id: row.get("subscription_id"),
            deployment_id: row.get("deployment_id"),
            service_endpoint: row.get("service_endpoint"),
            status: row.get("status"),
            allocated_at: row.get("allocated_at"),
            last_health_check: row.get("last_health_check"),
            cpu_hours: row.get("cpu_hours"),
            bandwidth_gb: row.get("bandwidth_gb"),
            storage_gb: row.get("storage_gb"),
            request_count: row.get("request_count"),
            pricing_tier: row.get("pricing_tier"),
            base_price_sats: row.get("base_price_sats"),
            usage_fees_sats: row.get("usage_fees_sats"),
            total_billed_sats: row.get("total_billed_sats"),
            last_invoice_date: row.get("last_invoice_date"),
        })
        .collect();
    
    Ok(slots)
}

pub async fn get_slot(pool: &PgPool, slot_id: &str) -> anyhow::Result<HostedSlot> {
    let slot_uuid = Uuid::parse_str(slot_id)?;
    
    let row = sqlx::query(
        "SELECT * FROM hosted_slots WHERE slot_id = $1"
    )
    .bind(slot_uuid)
    .fetch_one(pool)
    .await?;
    
    Ok(HostedSlot {
        slot_id: row.get("slot_id"),
        guardian_npub: row.get("guardian_npub"),
        federation_id: row.get("federation_id"),
        subscription_id: row.get("subscription_id"),
        deployment_id: row.get("deployment_id"),
        service_endpoint: row.get("service_endpoint"),
        status: row.get("status"),
        allocated_at: row.get("allocated_at"),
        last_health_check: row.get("last_health_check"),
        cpu_hours: row.get("cpu_hours"),
        bandwidth_gb: row.get("bandwidth_gb"),
        storage_gb: row.get("storage_gb"),
        request_count: row.get("request_count"),
        pricing_tier: row.get("pricing_tier"),
        base_price_sats: row.get("base_price_sats"),
        usage_fees_sats: row.get("usage_fees_sats"),
        total_billed_sats: row.get("total_billed_sats"),
        last_invoice_date: row.get("last_invoice_date"),
    })
}

pub async fn release_slot(pool: &PgPool, slot_id: &str) -> anyhow::Result<()> {
    let slot_uuid = Uuid::parse_str(slot_id)?;
    
    sqlx::query(
        "UPDATE hosted_slots SET status = 'released' WHERE slot_id = $1"
    )
    .bind(slot_uuid)
    .execute(pool)
    .await?;
    
    Ok(())
}

pub async fn count_active_slots(pool: &PgPool) -> anyhow::Result<u32> {
    let row = sqlx::query(
        "SELECT COUNT(*) as count FROM hosted_slots WHERE status IN ('allocated', 'running')"
    )
    .fetch_one(pool)
    .await?;
    
    let count: i64 = row.get("count");
    Ok(count as u32)
}

pub async fn get_current_pricing(pool: &PgPool) -> anyhow::Result<PricingInfo> {
    let row = sqlx::query(
        "SELECT * FROM pricing_history ORDER BY effective_date DESC LIMIT 1"
    )
    .fetch_one(pool)
    .await?;
    
    let resource_prices: serde_json::Value = row.get("resource_prices");
    
    Ok(PricingInfo {
        base_slot_price_sats: row.get::<i64, _>("base_slot_price_sats") as u64,
        cpu_per_core_hour_sats: resource_prices["cpu_per_core_hour_sats"].as_u64().unwrap_or(100),
        memory_per_gb_hour_sats: resource_prices["memory_per_gb_hour_sats"].as_u64().unwrap_or(50),
        storage_per_gb_month_sats: resource_prices["storage_per_gb_month_sats"].as_u64().unwrap_or(1000),
        bandwidth_per_gb_sats: resource_prices["bandwidth_per_gb_sats"].as_u64().unwrap_or(10),
    })
}

pub async fn get_revenue_report(pool: &PgPool, days: u32) -> anyhow::Result<RevenueReport> {
    // TODO: Implement actual revenue calculation
    Ok(RevenueReport {
        total_revenue_sats: 1000000,
        total_costs_sats: 600000,
        profit_sats: 400000,
        profit_margin_percent: 40.0,
    })
}

pub async fn get_utilization_report(pool: &PgPool) -> anyhow::Result<UtilizationReport> {
    let active_slots = count_active_slots(pool).await?;
    
    Ok(UtilizationReport {
        total_slots: 100, // TODO: Get from config
        active_slots,
        utilization_percent: (active_slots as f32 / 100.0) * 100.0,
        avg_cpu_percent: 45.0, // TODO: Calculate actual usage
        avg_memory_percent: 60.0,
    })
}

pub async fn update_base_price(pool: &PgPool, price: u64) -> anyhow::Result<()> {
    // TODO: Insert new pricing record
    Ok(())
}

pub async fn update_demand_multiplier(pool: &PgPool, multiplier: f32) -> anyhow::Result<()> {
    // TODO: Update demand curve in pricing
    Ok(())
}