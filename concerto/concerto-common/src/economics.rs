use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Pricing model for FeLaaS providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingModel {
    pub base_slot_price_sats: u64,
    pub resource_prices: ResourcePrices,
    pub demand_curve: Option<DemandCurve>,
    pub discount_tiers: Vec<DiscountTier>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePrices {
    pub cpu_per_core_hour_sats: u64,
    pub memory_per_gb_hour_sats: u64,
    pub storage_per_gb_month_sats: u64,
    pub bandwidth_per_gb_sats: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemandCurve {
    /// Multiplier when utilization is above this threshold
    pub high_demand_threshold: f32,
    pub high_demand_multiplier: f32,
    /// Discount when utilization is below this threshold
    pub low_demand_threshold: f32,
    pub low_demand_discount: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscountTier {
    pub min_slots: u32,
    pub discount_percent: f32,
}

/// Economic validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicValidation {
    pub subscription_valid: bool,
    pub tier_acceptable: bool,
    pub payment_verified: bool,
    pub credit_score: u32,
}

impl EconomicValidation {
    pub fn is_acceptable(&self) -> bool {
        self.subscription_valid && self.tier_acceptable && self.payment_verified
    }
}

/// Slot pricing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotPricing {
    pub price_sats: u64,
    pub billing_period: BillingPeriod,
    pub includes_resources: crate::ResourceBundle,
    pub additional_fees: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BillingPeriod {
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Annual,
}

/// Invoice for subscription billing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub invoice_id: uuid::Uuid,
    pub subscription_id: uuid::Uuid,
    pub period: BillingPeriod,
    pub base_fee_sats: u64,
    pub usage_fees_sats: u64,
    pub total_sats: u64,
    pub due_date: DateTime<Utc>,
    pub payment_request: Option<String>, // Lightning invoice
}

/// Provider offer for comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderOffer {
    pub provider_npub: String,
    pub provider_name: String,
    pub pricing: SlotPricing,
    pub reputation_score: f32,
    pub availability: f32,
    pub regions: Vec<String>,
    pub features: Vec<String>,
}

impl ProviderOffer {
    pub fn calculate_value_score(&self) -> u64 {
        // Lower score is better (price per quality point)
        let quality = (self.reputation_score * 100.0 + self.availability * 100.0) as u64;
        if quality == 0 {
            u64::MAX
        } else {
            self.pricing.price_sats / quality
        }
    }
}

/// Subscription tier for minimum requirements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum SubscriptionTier {
    Free,
    Basic,
    Standard,
    Premium,
    Enterprise,
}

/// Resource usage tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_hours: f32,
    pub bandwidth_gb: f32,
    pub storage_gb: f32,
    pub cost_sats: u64,
}

impl ResourceUsage {
    pub fn calculate_fees(&self, pricing: &ResourcePrices) -> u64 {
        let cpu_cost = (self.cpu_hours * pricing.cpu_per_core_hour_sats as f32) as u64;
        let bandwidth_cost = (self.bandwidth_gb * pricing.bandwidth_per_gb_sats as f32) as u64;
        let storage_cost = (self.storage_gb * pricing.storage_per_gb_month_sats as f32 / 30.0) as u64;
        
        cpu_cost + bandwidth_cost + storage_cost
    }
}

/// Provider economics tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEconomics {
    pub month: DateTime<Utc>,
    pub total_slots_hosted: u32,
    pub total_revenue_sats: u64,
    pub total_costs_sats: u64,
    pub profit_margin_percent: f32,
    pub average_utilization_percent: f32,
    pub peak_demand_multiplier: f32,
}

impl ProviderEconomics {
    pub fn calculate_profit(&self) -> i64 {
        self.total_revenue_sats as i64 - self.total_costs_sats as i64
    }

    pub fn is_profitable(&self) -> bool {
        self.total_revenue_sats > self.total_costs_sats
    }
}

/// Risk assessment for service providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub federation_id: String,
    pub risk_score: f32, // 0.0 (low risk) to 1.0 (high risk)
    pub factors: RiskFactors,
    pub accept: bool,
    pub suggested_terms: Option<ServiceTerms>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactors {
    pub guardian_diversity_score: f32,
    pub provider_diversity_score: f32,
    pub subscription_quality_score: f32,
    pub federation_age_days: u32,
    pub guardian_reputation_avg: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceTerms {
    pub fee_premium_percent: f32,
    pub collateral_required_sats: u64,
    pub min_commitment_days: u32,
}

impl PricingModel {
    pub fn standard() -> Self {
        Self {
            base_slot_price_sats: 100_000, // 0.001 BTC per slot per month
            resource_prices: ResourcePrices {
                cpu_per_core_hour_sats: 100,
                memory_per_gb_hour_sats: 50,
                storage_per_gb_month_sats: 1000,
                bandwidth_per_gb_sats: 10,
            },
            demand_curve: Some(DemandCurve {
                high_demand_threshold: 0.8,
                high_demand_multiplier: 1.5,
                low_demand_threshold: 0.3,
                low_demand_discount: 0.8,
            }),
            discount_tiers: vec![
                DiscountTier { min_slots: 10, discount_percent: 5.0 },
                DiscountTier { min_slots: 25, discount_percent: 10.0 },
                DiscountTier { min_slots: 50, discount_percent: 15.0 },
            ],
        }
    }

    pub fn calculate_price(&self, slots: u32, utilization: f32) -> u64 {
        let base_total = self.base_slot_price_sats * slots as u64;
        
        // Apply demand curve
        let demand_adjusted = if let Some(curve) = &self.demand_curve {
            if utilization > curve.high_demand_threshold {
                (base_total as f32 * curve.high_demand_multiplier) as u64
            } else if utilization < curve.low_demand_threshold {
                (base_total as f32 * curve.low_demand_discount) as u64
            } else {
                base_total
            }
        } else {
            base_total
        };
        
        // Apply volume discount
        let discount = self.discount_tiers
            .iter()
            .filter(|tier| slots >= tier.min_slots)
            .map(|tier| tier.discount_percent)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);
        
        let discounted = demand_adjusted - (demand_adjusted as f32 * discount / 100.0) as u64;
        
        discounted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pricing_model() {
        let pricing = PricingModel::standard();
        
        // Test base price (no demand adjustment, no discount)
        let price = pricing.calculate_price(1, 0.5);
        assert_eq!(price, 100_000);
        
        // Test high demand
        let high_demand_price = pricing.calculate_price(1, 0.9);
        assert_eq!(high_demand_price, 150_000); // 1.5x multiplier
        
        // Test low demand
        let low_demand_price = pricing.calculate_price(1, 0.2);
        assert_eq!(low_demand_price, 80_000); // 0.8x discount
        
        // Test volume discount (10 slots = 5% discount)
        let volume_price = pricing.calculate_price(10, 0.5);
        assert_eq!(volume_price, 950_000); // 1M - 5%
    }

    #[test]
    fn test_resource_usage_fees() {
        let pricing = ResourcePrices {
            cpu_per_core_hour_sats: 100,
            memory_per_gb_hour_sats: 50,
            storage_per_gb_month_sats: 3000,
            bandwidth_per_gb_sats: 10,
        };
        
        let usage = ResourceUsage {
            cpu_hours: 10.0,
            bandwidth_gb: 5.0,
            storage_gb: 10.0,
            cost_sats: 0,
        };
        
        let fees = usage.calculate_fees(&pricing);
        // CPU: 10 * 100 = 1000
        // Bandwidth: 5 * 10 = 50
        // Storage: 10 * 3000 / 30 = 1000
        assert_eq!(fees, 2050);
    }

    #[test]
    fn test_provider_offer_value_score() {
        let offer = ProviderOffer {
            provider_npub: "npub1test".to_string(),
            provider_name: "Test Provider".to_string(),
            pricing: SlotPricing {
                price_sats: 100_000,
                billing_period: BillingPeriod::Monthly,
                includes_resources: crate::ResourceBundle::standard(),
                additional_fees: HashMap::new(),
            },
            reputation_score: 0.9,
            availability: 0.95,
            regions: vec!["US".to_string()],
            features: vec![],
        };
        
        let score = offer.calculate_value_score();
        // Quality: (0.9 * 100 + 0.95 * 100) = 185
        // Score: 100000 / 185 = 540
        assert_eq!(score, 540);
    }
}