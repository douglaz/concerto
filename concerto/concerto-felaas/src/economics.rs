use concerto_common::*;
use tracing::{info, warn};

/// Calculate optimal pricing based on market conditions
pub struct PricingOptimizer {
    current_pricing: PricingModel,
    market_data: MarketData,
}

#[derive(Clone, Debug)]
pub struct MarketData {
    pub competitor_prices: Vec<u64>,
    pub demand_index: f32,
    pub supply_index: f32,
    pub bitcoin_price_usd: f32,
}

impl PricingOptimizer {
    pub fn new(current_pricing: PricingModel) -> Self {
        Self {
            current_pricing,
            market_data: MarketData {
                competitor_prices: vec![],
                demand_index: 1.0,
                supply_index: 1.0,
                bitcoin_price_usd: 50000.0,
            },
        }
    }
    
    pub fn optimize_pricing(&self, utilization: f32) -> PricingModel {
        let mut optimized = self.current_pricing.clone();
        
        // Adjust base price based on utilization
        if utilization > 0.8 {
            // High utilization - increase price
            optimized.base_slot_price_sats = 
                (optimized.base_slot_price_sats as f32 * 1.1) as u64;
            info!("Increasing prices due to high utilization: {}%", utilization * 100.0);
        } else if utilization < 0.3 {
            // Low utilization - decrease price
            optimized.base_slot_price_sats = 
                (optimized.base_slot_price_sats as f32 * 0.95) as u64;
            info!("Decreasing prices due to low utilization: {}%", utilization * 100.0);
        }
        
        // Adjust for market conditions
        if !self.market_data.competitor_prices.is_empty() {
            let avg_competitor_price: u64 = 
                self.market_data.competitor_prices.iter().sum::<u64>() / 
                self.market_data.competitor_prices.len() as u64;
            
            if optimized.base_slot_price_sats > avg_competitor_price * 2 {
                warn!("Price significantly above market average");
                optimized.base_slot_price_sats = 
                    (avg_competitor_price as f32 * 1.5) as u64;
            }
        }
        
        optimized
    }
    
    pub fn calculate_roi(&self, revenue: u64, costs: u64) -> f32 {
        if costs == 0 {
            return 0.0;
        }
        ((revenue as f32 - costs as f32) / costs as f32) * 100.0
    }
}

/// Risk assessment for accepting new federations
pub struct RiskAssessor;

impl RiskAssessor {
    pub fn assess_federation(
        federation_id: &str,
        guardian_count: u32,
        subscription_tiers: &[SubscriptionTier],
    ) -> RiskAssessment {
        let mut risk_score = 0.0;
        let mut factors = RiskFactors {
            guardian_diversity_score: 0.0,
            provider_diversity_score: 0.0,
            subscription_quality_score: 0.0,
            federation_age_days: 0,
            guardian_reputation_avg: 0.0,
        };
        
        // Guardian diversity assessment
        if guardian_count < 3 {
            risk_score += 0.3;
            factors.guardian_diversity_score = 0.3;
        } else if guardian_count > 7 {
            risk_score -= 0.1;
            factors.guardian_diversity_score = 0.9;
        } else {
            factors.guardian_diversity_score = 0.7;
        }
        
        // Subscription quality assessment
        let avg_tier = subscription_tiers.len() as f32 / guardian_count as f32;
        factors.subscription_quality_score = avg_tier.min(1.0);
        
        if avg_tier < 0.5 {
            risk_score += 0.2;
        }
        
        // Overall risk assessment
        let accept = risk_score < 0.5;
        
        let suggested_terms = if risk_score > 0.3 {
            Some(ServiceTerms {
                fee_premium_percent: risk_score * 20.0,
                collateral_required_sats: (100000.0 * risk_score) as u64,
                min_commitment_days: 30,
            })
        } else {
            None
        };
        
        RiskAssessment {
            federation_id: federation_id.to_string(),
            risk_score,
            factors,
            accept,
            suggested_terms,
        }
    }
    
    pub fn assess_guardian(
        guardian_npub: &str,
        subscription: &Subscription,
        history: Option<&GuardianHistory>,
    ) -> GuardianRiskScore {
        let mut score = 0.5; // Neutral starting point
        
        // Check subscription status
        if subscription.is_active() {
            score -= 0.1;
        } else {
            score += 0.2;
        }
        
        // Check history if available
        if let Some(hist) = history {
            if hist.successful_federations > 0 {
                score -= 0.1 * (hist.successful_federations.min(5) as f32 / 5.0);
            }
            if hist.failed_federations > 0 {
                score += 0.1 * (hist.failed_federations as f32);
            }
        }
        
        GuardianRiskScore {
            guardian_npub: guardian_npub.to_string(),
            risk_score: score.max(0.0).min(1.0),
            factors: vec![
                "subscription_status".to_string(),
                "federation_history".to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug)]
pub struct GuardianHistory {
    pub successful_federations: u32,
    pub failed_federations: u32,
    pub total_slots_managed: u32,
    pub average_uptime_percent: f32,
}

#[derive(Clone, Debug)]
pub struct GuardianRiskScore {
    pub guardian_npub: String,
    pub risk_score: f32,
    pub factors: Vec<String>,
}

/// Revenue tracking and reporting
pub struct RevenueTracker;

impl RevenueTracker {
    pub fn calculate_monthly_revenue(
        active_slots: u32,
        base_price: u64,
        usage_fees: u64,
    ) -> u64 {
        (active_slots as u64 * base_price) + usage_fees
    }
    
    pub fn calculate_costs(
        cpu_hours: f32,
        memory_gb_hours: f32,
        storage_gb_months: f32,
        bandwidth_gb: f32,
    ) -> u64 {
        // Estimated infrastructure costs
        let cpu_cost = (cpu_hours * 50.0) as u64; // 50 sats per CPU hour
        let memory_cost = (memory_gb_hours * 25.0) as u64; // 25 sats per GB hour
        let storage_cost = (storage_gb_months * 500.0) as u64; // 500 sats per GB month
        let bandwidth_cost = (bandwidth_gb * 5.0) as u64; // 5 sats per GB
        
        cpu_cost + memory_cost + storage_cost + bandwidth_cost
    }
    
    pub fn calculate_profit_margin(revenue: u64, costs: u64) -> f32 {
        if revenue == 0 {
            return 0.0;
        }
        ((revenue as f32 - costs as f32) / revenue as f32) * 100.0
    }
}