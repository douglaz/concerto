use concerto_common::*;
use chrono::{Utc, Duration};
use uuid::Uuid;

#[test]
fn test_subscription_creation() -> anyhow::Result<()> {
    let sub = Subscription {
        id: Uuid::new_v4(),
        owner_npub: "npub1test".to_string(),
        plan: SubscriptionPlan {
            name: "Professional".to_string(),
            tier: SubscriptionTier::Professional,
            fedimint_slots: 10,
            price_sats: 100000,
            duration_days: 30,
        },
        payment_info: PaymentInfo {
            invoice: "lnbc...".to_string(),
            payment_hash: "hash123".to_string(),
            paid_at: Some(Utc::now()),
            status: PaymentStatus::Paid,
        },
        valid_until: Utc::now() + Duration::days(30),
        allocated_slots: 5,
        created_at: Utc::now(),
    };

    assert_eq!(sub.available_slots(), 5);
    assert!(sub.is_valid());
    assert!(sub.can_allocate_slots(3));
    assert!(!sub.can_allocate_slots(10));

    Ok(())
}

#[test]
fn test_subscription_proof_generation() -> anyhow::Result<()> {
    let proof = SubscriptionProof {
        subscription_id: Uuid::new_v4(),
        owner_npub: "npub1owner".to_string(),
        provider_npub: "npub1provider".to_string(),
        valid_until: Utc::now() + Duration::days(30),
        slots_allocated: 2,
        signature: "signature123".to_string(),
    };

    let json = serde_json::to_string(&proof)?;
    let decoded: SubscriptionProof = serde_json::from_str(&json)?;
    
    assert_eq!(decoded.owner_npub, proof.owner_npub);
    assert_eq!(decoded.slots_allocated, proof.slots_allocated);
    
    Ok(())
}

#[test]
fn test_subscription_tiers() -> anyhow::Result<()> {
    assert_eq!(SubscriptionTier::Basic.slots(), 1);
    assert_eq!(SubscriptionTier::Professional.slots(), 5);
    assert_eq!(SubscriptionTier::Enterprise.slots(), 20);
    assert_eq!(SubscriptionTier::Custom { slots: 42 }.slots(), 42);
    
    Ok(())
}

#[test]
fn test_payment_status_transitions() -> anyhow::Result<()> {
    let mut payment = PaymentInfo {
        invoice: "lnbc1...".to_string(),
        payment_hash: "hash456".to_string(),
        paid_at: None,
        status: PaymentStatus::Pending,
    };
    
    assert!(matches!(payment.status, PaymentStatus::Pending));
    
    // Simulate payment completion
    payment.status = PaymentStatus::Paid;
    payment.paid_at = Some(Utc::now());
    
    assert!(matches!(payment.status, PaymentStatus::Paid));
    assert!(payment.paid_at.is_some());
    
    Ok(())
}

#[test]
fn test_subscription_expiry() -> anyhow::Result<()> {
    let expired_sub = Subscription {
        id: Uuid::new_v4(),
        owner_npub: "npub1expired".to_string(),
        plan: SubscriptionPlan {
            name: "Basic".to_string(),
            tier: SubscriptionTier::Basic,
            fedimint_slots: 1,
            price_sats: 10000,
            duration_days: 30,
        },
        payment_info: PaymentInfo {
            invoice: "lnbc...".to_string(),
            payment_hash: "hash789".to_string(),
            paid_at: Some(Utc::now() - Duration::days(60)),
            status: PaymentStatus::Paid,
        },
        valid_until: Utc::now() - Duration::days(10), // Expired 10 days ago
        allocated_slots: 0,
        created_at: Utc::now() - Duration::days(60),
    };
    
    assert!(!expired_sub.is_valid());
    assert!(!expired_sub.can_allocate_slots(1));
    
    Ok(())
}

#[test]
fn test_subscription_plan_serialization() -> anyhow::Result<()> {
    let plan = SubscriptionPlan {
        name: "Custom Plan".to_string(),
        tier: SubscriptionTier::Custom { slots: 15 },
        fedimint_slots: 15,
        price_sats: 150000,
        duration_days: 90,
    };
    
    let json = serde_json::to_string(&plan)?;
    let decoded: SubscriptionPlan = serde_json::from_str(&json)?;
    
    assert_eq!(decoded.name, plan.name);
    assert_eq!(decoded.fedimint_slots, plan.fedimint_slots);
    assert_eq!(decoded.price_sats, plan.price_sats);
    assert_eq!(decoded.duration_days, plan.duration_days);
    
    Ok(())
}