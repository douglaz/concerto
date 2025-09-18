use concerto_common::*;
use nostr_sdk::prelude::*;

#[test]
fn test_federation_proposal_event() -> anyhow::Result<()> {
    let proposal = FederationProposalEvent {
        federation_id: "fed_test_123".to_string(),
        name: "Test Federation".to_string(),
        description: Some("A test federation for unit testing".to_string()),
        initiator_slots: 2,
        total_slots: 4,
        requirements: FederationRequirements {
            min_slots: 1,
            min_subscription_tier: None,
            required_features: vec!["lightning".to_string()],
            geographic_diversity: None,
            custom_requirements: std::collections::HashMap::new(),
        },
        consensus_config: ConsensusConfig::standard(4),
    };
    
    // Test serialization
    let json = serde_json::to_string(&proposal)?;
    let decoded: FederationProposalEvent = serde_json::from_str(&json)?;
    
    assert_eq!(decoded.federation_id, proposal.federation_id);
    assert_eq!(decoded.name, proposal.name);
    assert_eq!(decoded.initiator_slots, proposal.initiator_slots);
    assert_eq!(decoded.total_slots, proposal.total_slots);
    
    Ok(())
}

#[test]
fn test_guardian_application_event() -> anyhow::Result<()> {
    let application = GuardianApplicationEvent {
        federation_id: "fed_apply_test".to_string(),
        applicant_npub: "npub1applicant".to_string(),
        slots_to_contribute: 2,
        preferred_providers: vec![
            url::Url::parse("https://provider1.example.com")?,
            url::Url::parse("https://provider2.example.com")?,
        ],
        message: Some("I'd like to join this federation".to_string()),
        subscription_proof: Some(SubscriptionProof {
            subscription_id: uuid::Uuid::new_v4(),
            owner_npub: "npub1applicant".to_string(),
            provider_npub: "npub1provider".to_string(),
            valid_until: chrono::Utc::now() + chrono::Duration::days(30),
            slots_allocated: 2,
            signature: "sig123".to_string(),
        }),
    };
    
    let json = serde_json::to_string(&application)?;
    let decoded: GuardianApplicationEvent = serde_json::from_str(&json)?;
    
    assert_eq!(decoded.federation_id, application.federation_id);
    assert_eq!(decoded.applicant_npub, application.applicant_npub);
    assert_eq!(decoded.slots_to_contribute, 2);
    assert_eq!(decoded.preferred_providers.len(), 2);
    assert!(decoded.subscription_proof.is_some());
    
    Ok(())
}

#[test]
fn test_application_decision_event() -> anyhow::Result<()> {
    let approved = ApplicationDecisionEvent {
        application_event_id: "event123".to_string(),
        federation_id: "fed_decision_test".to_string(),
        applicant_npub: "npub1applicant".to_string(),
        decision: Decision::Approved,
        message: Some("Welcome to the federation!".to_string()),
    };
    
    let rejected = ApplicationDecisionEvent {
        application_event_id: "event456".to_string(),
        federation_id: "fed_decision_test".to_string(),
        applicant_npub: "npub1applicant2".to_string(),
        decision: Decision::Rejected {
            reason: "Insufficient slots".to_string(),
        },
        message: Some("Sorry, we need more slots".to_string()),
    };
    
    // Test approved decision
    let json = serde_json::to_string(&approved)?;
    let decoded: ApplicationDecisionEvent = serde_json::from_str(&json)?;
    assert!(matches!(decoded.decision, Decision::Approved));
    
    // Test rejected decision
    let json = serde_json::to_string(&rejected)?;
    let decoded: ApplicationDecisionEvent = serde_json::from_str(&json)?;
    if let Decision::Rejected { reason } = decoded.decision {
        assert_eq!(reason, "Insufficient slots");
    } else {
        panic!("Expected rejected decision");
    }
    
    Ok(())
}

#[test]
fn test_slot_allocation_event() -> anyhow::Result<()> {
    let allocation = SlotAllocationEvent {
        federation_id: "fed_slot_test".to_string(),
        slot_id: uuid::Uuid::new_v4(),
        guardian_npub: "npub1guardian".to_string(),
        provider_url: url::Url::parse("https://provider.example.com")?,
        endpoint: Some(url::Url::parse("http://localhost:8173")?),
        status: SlotState::Allocated {
            federation_id: "fed_slot_test".to_string(),
        },
    };
    
    let json = serde_json::to_string(&allocation)?;
    let decoded: SlotAllocationEvent = serde_json::from_str(&json)?;
    
    assert_eq!(decoded.federation_id, allocation.federation_id);
    assert_eq!(decoded.guardian_npub, allocation.guardian_npub);
    assert!(decoded.endpoint.is_some());
    
    Ok(())
}

#[test]
fn test_dkg_coordination_event() -> anyhow::Result<()> {
    let initiate = DkgCoordinationEvent {
        federation_id: "fed_dkg_test".to_string(),
        round: 0,
        message_type: DkgMessageType::Initiate {
            participants: vec![
                "npub1p1".to_string(),
                "npub1p2".to_string(),
                "npub1p3".to_string(),
            ],
        },
        payload: "initiation_payload".to_string(),
    };
    
    let share = DkgCoordinationEvent {
        federation_id: "fed_dkg_test".to_string(),
        round: 1,
        message_type: DkgMessageType::Share {
            from: "npub1p1".to_string(),
            round: 1,
        },
        payload: "encrypted_share".to_string(),
    };
    
    let complete = DkgCoordinationEvent {
        federation_id: "fed_dkg_test".to_string(),
        round: 3,
        message_type: DkgMessageType::Complete { success: true },
        payload: "final_public_key".to_string(),
    };
    
    // Test initiate
    let json = serde_json::to_string(&initiate)?;
    let decoded: DkgCoordinationEvent = serde_json::from_str(&json)?;
    if let DkgMessageType::Initiate { participants } = decoded.message_type {
        assert_eq!(participants.len(), 3);
    }
    
    // Test share
    let json = serde_json::to_string(&share)?;
    let decoded: DkgCoordinationEvent = serde_json::from_str(&json)?;
    if let DkgMessageType::Share { from, round } = decoded.message_type {
        assert_eq!(from, "npub1p1");
        assert_eq!(round, 1);
    }
    
    // Test complete
    let json = serde_json::to_string(&complete)?;
    let decoded: DkgCoordinationEvent = serde_json::from_str(&json)?;
    if let DkgMessageType::Complete { success } = decoded.message_type {
        assert!(success);
    }
    
    Ok(())
}

#[test]
fn test_service_advertisement_event() -> anyhow::Result<()> {
    let felaas_ad = ServiceAdvertisementEvent {
        provider_npub: "npub1provider".to_string(),
        service_type: ServiceType::FeLaaSProvider {
            regions: vec!["US-East".to_string(), "EU-West".to_string()],
            pricing: PricingModel::default(),
        },
        terms: ServiceTerms {
            min_federation_size: 3,
            fee_structure: FeeStructure::Flat { sats_per_month: 100000 },
            availability: ServiceAvailability::Always,
        },
        supported_federations: Some(vec!["fed1".to_string(), "fed2".to_string()]),
        requirements: vec!["lightning".to_string(), "onchain".to_string()],
    };
    
    let lightning_ad = ServiceAdvertisementEvent {
        provider_npub: "npub1lightning".to_string(),
        service_type: ServiceType::LightningGateway {
            capacity_sats: 10000000,
            fee_rate: 0.001,
        },
        terms: ServiceTerms {
            min_federation_size: 1,
            fee_structure: FeeStructure::Percentage { basis_points: 10 },
            availability: ServiceAvailability::Hours { start: 9, end: 17 },
        },
        supported_federations: None,
        requirements: vec![],
    };
    
    // Test FeLaaS provider
    let json = serde_json::to_string(&felaas_ad)?;
    let decoded: ServiceAdvertisementEvent = serde_json::from_str(&json)?;
    if let ServiceType::FeLaaSProvider { regions, .. } = decoded.service_type {
        assert_eq!(regions.len(), 2);
    }
    
    // Test Lightning gateway
    let json = serde_json::to_string(&lightning_ad)?;
    let decoded: ServiceAdvertisementEvent = serde_json::from_str(&json)?;
    if let ServiceType::LightningGateway { capacity_sats, fee_rate } = decoded.service_type {
        assert_eq!(capacity_sats, 10000000);
        assert_eq!(fee_rate, 0.001);
    }
    
    Ok(())
}

#[test]
fn test_service_types() -> anyhow::Result<()> {
    let stability_pool = ServiceType::StabilityPool {
        available_liquidity: 50000000,
        collateral_ratio: 1.5,
    };
    
    let onchain = ServiceType::OnchainServices {
        services: vec![
            "coinjoin".to_string(),
            "payjoin".to_string(),
            "batching".to_string(),
        ],
    };
    
    // Test stability pool
    if let ServiceType::StabilityPool { available_liquidity, collateral_ratio } = stability_pool {
        assert_eq!(available_liquidity, 50000000);
        assert_eq!(collateral_ratio, 1.5);
    }
    
    // Test onchain services
    if let ServiceType::OnchainServices { services } = onchain {
        assert_eq!(services.len(), 3);
        assert!(services.contains(&"coinjoin".to_string()));
    }
    
    Ok(())
}

#[test]
fn test_fee_structures() -> anyhow::Result<()> {
    let flat = FeeStructure::Flat { sats_per_month: 50000 };
    let percentage = FeeStructure::Percentage { basis_points: 25 };
    let tiered = FeeStructure::Tiered {
        tiers: vec![
            PriceTier { up_to_amount_sats: 100000, fee_basis_points: 100 },
            PriceTier { up_to_amount_sats: 1000000, fee_basis_points: 75 },
            PriceTier { up_to_amount_sats: 10000000, fee_basis_points: 50 },
        ],
    };
    
    // Test flat fee
    if let FeeStructure::Flat { sats_per_month } = flat {
        assert_eq!(sats_per_month, 50000);
    }
    
    // Test percentage fee
    if let FeeStructure::Percentage { basis_points } = percentage {
        assert_eq!(basis_points, 25);
    }
    
    // Test tiered fee
    if let FeeStructure::Tiered { tiers } = tiered {
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].fee_basis_points, 100);
        assert_eq!(tiers[2].up_to_amount_sats, 10000000);
    }
    
    Ok(())
}

#[test]
fn test_event_kind_constants() -> anyhow::Result<()> {
    assert_eq!(KIND_FEDERATION_PROPOSAL, 30500);
    assert_eq!(KIND_GUARDIAN_APPLICATION, 30501);
    assert_eq!(KIND_APPLICATION_DECISION, 30502);
    assert_eq!(KIND_SLOT_ALLOCATION, 30503);
    assert_eq!(KIND_DKG_COORDINATION, 30504);
    assert_eq!(KIND_SERVICE_ADVERTISEMENT, 30600);
    
    Ok(())
}