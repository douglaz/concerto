use concerto_common::*;
use chrono::Utc;
use std::collections::HashMap;

#[test]
fn test_federation_creation() -> anyhow::Result<()> {
    let fed = Federation {
        id: "fed_test123".to_string(),
        name: "Test Federation".to_string(),
        description: Some("A test federation for unit testing".to_string()),
        status: FederationStatus::Proposed,
        initiator_npub: "npub1initiator".to_string(),
        guardians: vec![
            Guardian {
                npub: "npub1initiator".to_string(),
                name: Some("Alice".to_string()),
                slots_contributed: 2,
                role: GuardianRole::LeadGuardian,
                joined_at: Utc::now(),
            },
        ],
        total_slots: 4,
        allocated_slots: 2,
        requirements: FederationRequirements {
            min_slots: 1,
            min_subscription_tier: Some(SubscriptionTier::Professional),
            required_features: vec!["lightning".to_string()],
            geographic_diversity: Some(GeographicRequirement {
                min_regions: 2,
                max_per_region: 2,
            }),
            custom_requirements: HashMap::new(),
        },
        consensus_config: ConsensusConfig::standard(4),
        created_at: Utc::now(),
        activated_at: None,
    };

    assert_eq!(fed.status, FederationStatus::Proposed);
    assert_eq!(fed.available_slots(), 2);
    assert!(fed.needs_more_guardians());
    
    Ok(())
}

#[test]
fn test_federation_status_transitions() -> anyhow::Result<()> {
    let mut fed = Federation {
        id: "fed_status_test".to_string(),
        name: "Status Test Federation".to_string(),
        description: None,
        status: FederationStatus::Proposed,
        initiator_npub: "npub1init".to_string(),
        guardians: vec![],
        total_slots: 4,
        allocated_slots: 0,
        requirements: FederationRequirements::default(),
        consensus_config: ConsensusConfig::standard(4),
        created_at: Utc::now(),
        activated_at: None,
    };
    
    // Progress through states
    assert!(matches!(fed.status, FederationStatus::Proposed));
    
    fed.status = FederationStatus::Forming;
    assert!(matches!(fed.status, FederationStatus::Forming));
    
    fed.status = FederationStatus::ConfiguringDKG;
    assert!(matches!(fed.status, FederationStatus::ConfiguringDKG));
    
    fed.status = FederationStatus::Active;
    fed.activated_at = Some(Utc::now());
    assert!(matches!(fed.status, FederationStatus::Active));
    assert!(fed.activated_at.is_some());
    
    Ok(())
}

#[test]
fn test_guardian_roles() -> anyhow::Result<()> {
    let lg = Guardian {
        npub: "npub1lead".to_string(),
        name: Some("Lead Guardian".to_string()),
        slots_contributed: 3,
        role: GuardianRole::LeadGuardian,
        joined_at: Utc::now(),
    };
    
    let og = Guardian {
        npub: "npub1other".to_string(),
        name: Some("Other Guardian".to_string()),
        slots_contributed: 1,
        role: GuardianRole::OtherGuardian,
        joined_at: Utc::now(),
    };
    
    assert!(matches!(lg.role, GuardianRole::LeadGuardian));
    assert!(matches!(og.role, GuardianRole::OtherGuardian));
    assert_eq!(lg.slots_contributed, 3);
    assert_eq!(og.slots_contributed, 1);
    
    Ok(())
}

#[test]
fn test_consensus_config() -> anyhow::Result<()> {
    let config = ConsensusConfig::standard(7);
    assert_eq!(config.threshold, 5); // (7/2) + 1
    assert_eq!(config.total_guardians, 7);
    
    let small_config = ConsensusConfig::standard(3);
    assert_eq!(small_config.threshold, 2); // (3/2) + 1
    
    let large_config = ConsensusConfig::standard(10);
    assert_eq!(large_config.threshold, 6); // (10/2) + 1
    
    Ok(())
}

#[test]
fn test_federation_requirements() -> anyhow::Result<()> {
    let mut requirements = FederationRequirements {
        min_slots: 2,
        min_subscription_tier: Some(SubscriptionTier::Professional),
        required_features: vec!["lightning".to_string(), "onchain".to_string()],
        geographic_diversity: Some(GeographicRequirement {
            min_regions: 3,
            max_per_region: 2,
        }),
        custom_requirements: HashMap::new(),
    };
    
    requirements.custom_requirements.insert(
        "uptime".to_string(),
        "99.9%".to_string()
    );
    
    assert_eq!(requirements.min_slots, 2);
    assert_eq!(requirements.required_features.len(), 2);
    assert!(requirements.geographic_diversity.is_some());
    assert_eq!(
        requirements.custom_requirements.get("uptime"),
        Some(&"99.9%".to_string())
    );
    
    Ok(())
}

#[test]
fn test_decision_enum() -> anyhow::Result<()> {
    let approved = Decision::Approved;
    let rejected = Decision::Rejected {
        reason: "Insufficient slots".to_string(),
    };
    
    assert!(matches!(approved, Decision::Approved));
    
    if let Decision::Rejected { reason } = rejected {
        assert_eq!(reason, "Insufficient slots");
    } else {
        panic!("Expected Rejected decision");
    }
    
    Ok(())
}

#[test]
fn test_federation_serialization() -> anyhow::Result<()> {
    let fed = Federation {
        id: "fed_ser_test".to_string(),
        name: "Serialization Test".to_string(),
        description: Some("Testing JSON serialization".to_string()),
        status: FederationStatus::Active,
        initiator_npub: "npub1init".to_string(),
        guardians: vec![
            Guardian {
                npub: "npub1g1".to_string(),
                name: Some("Guardian 1".to_string()),
                slots_contributed: 2,
                role: GuardianRole::LeadGuardian,
                joined_at: Utc::now(),
            },
            Guardian {
                npub: "npub1g2".to_string(),
                name: None,
                slots_contributed: 1,
                role: GuardianRole::OtherGuardian,
                joined_at: Utc::now(),
            },
        ],
        total_slots: 4,
        allocated_slots: 3,
        requirements: FederationRequirements::default(),
        consensus_config: ConsensusConfig::standard(4),
        created_at: Utc::now(),
        activated_at: Some(Utc::now()),
    };
    
    let json = serde_json::to_string(&fed)?;
    let decoded: Federation = serde_json::from_str(&json)?;
    
    assert_eq!(decoded.id, fed.id);
    assert_eq!(decoded.name, fed.name);
    assert_eq!(decoded.guardians.len(), 2);
    assert_eq!(decoded.status, FederationStatus::Active);
    
    Ok(())
}