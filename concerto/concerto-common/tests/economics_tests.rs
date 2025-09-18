use concerto_common::*;
use chrono::{Utc, Duration};

#[test]
fn test_pricing_model() -> anyhow::Result<()> {
    let pricing = PricingModel {
        base_price_sats: 100000,
        price_per_slot_sats: 20000,
        volume_discounts: vec![
            VolumeDiscount { min_slots: 5, discount_percent: 10.0 },
            VolumeDiscount { min_slots: 10, discount_percent: 20.0 },
        ],
        surge_pricing: Some(SurgePricing {
            enabled: true,
            multiplier: 1.5,
            threshold_utilization: 0.8,
        }),
        custom_pricing_rules: std::collections::HashMap::new(),
    };
    
    // Test base pricing
    let price_1_slot = pricing.calculate_price(1, 0.5);
    assert_eq!(price_1_slot, 120000); // base + 1 slot
    
    // Test volume discount
    let price_5_slots = pricing.calculate_price(5, 0.5);
    let expected_5 = ((100000 + 5 * 20000) as f64 * 0.9) as u64;
    assert_eq!(price_5_slots, expected_5);
    
    // Test surge pricing
    let price_surge = pricing.calculate_price(1, 0.85);
    let expected_surge = ((100000 + 20000) as f64 * 1.5) as u64;
    assert_eq!(price_surge, expected_surge);
    
    Ok(())
}

#[test]
fn test_billing_record() -> anyhow::Result<()> {
    let billing = BillingRecord {
        subscription_id: uuid::Uuid::new_v4(),
        period_start: Utc::now() - Duration::days(30),
        period_end: Utc::now(),
        total_amount_sats: 150000,
        resource_usage: ResourceUsage {
            timestamp: Utc::now(),
            cpu_percent: 75.0,
            memory_mb_used: 2048,
            storage_gb_used: 50.0,
            bandwidth_mbps_current: 25.0,
            requests_per_second: Some(1000.0),
        },
        invoice: "lnbc150000...".to_string(),
        paid: true,
    };
    
    assert_eq!(billing.total_amount_sats, 150000);
    assert!(billing.paid);
    assert_eq!(billing.resource_usage.cpu_percent, 75.0);
    
    Ok(())
}

#[test]
fn test_risk_assessment() -> anyhow::Result<()> {
    let assessment = RiskAssessment {
        federation_id: "fed_risk".to_string(),
        risk_score: 3.5,
        factors: vec![
            RiskFactor {
                name: "Guardian Count".to_string(),
                weight: 0.3,
                score: 4.0,
                description: "Adequate number of guardians".to_string(),
            },
            RiskFactor {
                name: "Geographic Distribution".to_string(),
                weight: 0.2,
                score: 3.0,
                description: "Moderate geographic diversity".to_string(),
            },
        ],
        recommendation: RiskRecommendation::Accept,
        max_recommended_slots: Some(5),
    };
    
    assert_eq!(assessment.risk_score, 3.5);
    assert_eq!(assessment.factors.len(), 2);
    assert!(matches!(assessment.recommendation, RiskRecommendation::Accept));
    assert_eq!(assessment.max_recommended_slots, Some(5));
    
    Ok(())
}

#[test]
fn test_risk_recommendation() -> anyhow::Result<()> {
    let accept = RiskRecommendation::Accept;
    let review = RiskRecommendation::RequireReview { 
        reason: "High resource usage".to_string() 
    };
    let reject = RiskRecommendation::Reject { 
        reason: "Insufficient guardians".to_string() 
    };
    
    assert!(matches!(accept, RiskRecommendation::Accept));
    
    if let RiskRecommendation::RequireReview { reason } = review {
        assert_eq!(reason, "High resource usage");
    }
    
    if let RiskRecommendation::Reject { reason } = reject {
        assert_eq!(reason, "Insufficient guardians");
    }
    
    Ok(())
}

#[test]
fn test_revenue_report() -> anyhow::Result<()> {
    let report = RevenueReport {
        period_start: Utc::now() - Duration::days(30),
        period_end: Utc::now(),
        total_revenue_sats: 5000000,
        active_subscriptions: 25,
        new_subscriptions: 5,
        churned_subscriptions: 2,
        average_revenue_per_user: 200000,
        utilization_rate: 0.75,
        growth_rate: 0.15,
    };
    
    assert_eq!(report.total_revenue_sats, 5000000);
    assert_eq!(report.active_subscriptions, 25);
    assert_eq!(report.net_new_subscriptions(), 3);
    assert_eq!(report.utilization_rate, 0.75);
    
    Ok(())
}

#[test]
fn test_economic_validation() -> anyhow::Result<()> {
    let validation = EconomicValidation {
        is_valid: true,
        subscription_active: true,
        slots_available: true,
        payment_verified: true,
        risk_acceptable: true,
        messages: vec!["All checks passed".to_string()],
    };
    
    assert!(validation.is_valid);
    assert!(validation.subscription_active);
    assert!(validation.slots_available);
    assert!(validation.payment_verified);
    assert!(validation.risk_acceptable);
    assert_eq!(validation.messages.len(), 1);
    
    Ok(())
}

#[test]
fn test_slot_pricing() -> anyhow::Result<()> {
    let pricing = SlotPricing {
        slot_id: uuid::Uuid::new_v4(),
        base_price_sats: 50000,
        current_price_sats: 75000, // With surge
        discount_applied: None,
        surge_multiplier: Some(1.5),
        estimated_monthly_cost: 75000 * 30,
    };
    
    assert_eq!(pricing.base_price_sats, 50000);
    assert_eq!(pricing.current_price_sats, 75000);
    assert_eq!(pricing.surge_multiplier, Some(1.5));
    assert_eq!(pricing.estimated_monthly_cost, 2250000);
    
    Ok(())
}

#[test]
fn test_provider_economics() -> anyhow::Result<()> {
    let economics = ProviderEconomics {
        provider_npub: "npub1provider".to_string(),
        total_revenue_sats: 10000000,
        total_costs_sats: 6000000,
        profit_margin: 0.4,
        active_slots: 50,
        total_capacity: 100,
        utilization_rate: 0.5,
        average_price_per_slot: 200000,
        top_federations: vec![
            ("fed_1".to_string(), 1000000),
            ("fed_2".to_string(), 800000),
        ],
    };
    
    assert_eq!(economics.net_profit_sats(), 4000000);
    assert_eq!(economics.profit_margin, 0.4);
    assert_eq!(economics.utilization_rate, 0.5);
    assert_eq!(economics.top_federations.len(), 2);
    
    Ok(())
}

#[test]
fn test_economic_event() -> anyhow::Result<()> {
    let event = EconomicEvent {
        timestamp: Utc::now(),
        event_type: EconomicEventType::SubscriptionPurchased {
            subscription_id: uuid::Uuid::new_v4(),
            amount_sats: 100000,
        },
        provider_npub: "npub1provider".to_string(),
        impact_sats: 100000,
    };
    
    if let EconomicEventType::SubscriptionPurchased { amount_sats, .. } = event.event_type {
        assert_eq!(amount_sats, 100000);
    }
    
    assert_eq!(event.impact_sats, 100000);
    
    Ok(())
}

#[test]
fn test_service_terms() -> anyhow::Result<()> {
    let terms = ServiceTerms {
        minimum_duration_days: 30,
        maximum_duration_days: 365,
        cancellation_policy: "30 days notice required".to_string(),
        refund_policy: "Pro-rated refunds available".to_string(),
        sla_guarantees: vec![
            "99.9% uptime".to_string(),
            "24/7 support".to_string(),
        ],
        payment_methods: vec![
            "Lightning".to_string(),
            "On-chain Bitcoin".to_string(),
        ],
    };
    
    assert_eq!(terms.minimum_duration_days, 30);
    assert_eq!(terms.sla_guarantees.len(), 2);
    assert_eq!(terms.payment_methods.len(), 2);
    
    Ok(())
}