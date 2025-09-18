use concerto_common::*;
use chrono::{Utc, Duration};
use uuid::Uuid;
use url::Url;

#[test]
fn test_slot_creation() -> anyhow::Result<()> {
    let slot = FedimintSlot {
        id: Uuid::new_v4(),
        owner_npub: "npub1owner".to_string(),
        subscription_id: Uuid::new_v4(),
        state: SlotState::Available,
        federation_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    assert!(matches!(slot.state, SlotState::Available));
    assert!(slot.federation_id.is_none());
    assert!(slot.is_available());
    
    Ok(())
}

#[test]
fn test_slot_state_transitions() -> anyhow::Result<()> {
    let mut slot = FedimintSlot {
        id: Uuid::new_v4(),
        owner_npub: "npub1owner".to_string(),
        subscription_id: Uuid::new_v4(),
        state: SlotState::Available,
        federation_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    // Allocate slot
    slot.state = SlotState::Allocated {
        federation_id: "fed_123".to_string(),
    };
    slot.federation_id = Some("fed_123".to_string());
    assert!(!slot.is_available());
    
    // Launch slot
    slot.state = SlotState::Launching {
        federation_id: "fed_123".to_string(),
    };
    
    // Slot running
    slot.state = SlotState::Running {
        federation_id: "fed_123".to_string(),
        endpoint: Url::parse("http://localhost:8173")?,
        health_status: HealthStatus::Healthy {
            last_check: Utc::now(),
            uptime_seconds: 3600,
        },
    };
    
    if let SlotState::Running { health_status, .. } = &slot.state {
        assert!(matches!(health_status, HealthStatus::Healthy { .. }));
    }
    
    Ok(())
}

#[test]
fn test_slot_allocation() -> anyhow::Result<()> {
    let allocation = SlotAllocation {
        slot_id: Uuid::new_v4(),
        provider_npub: "npub1provider".to_string(),
        allocated_at: Utc::now(),
        expires_at: Utc::now() + Duration::days(30),
    };
    
    assert!(!allocation.is_expired());
    
    // Test expired allocation
    let expired = SlotAllocation {
        slot_id: Uuid::new_v4(),
        provider_npub: "npub1provider".to_string(),
        allocated_at: Utc::now() - Duration::days(40),
        expires_at: Utc::now() - Duration::days(10),
    };
    
    assert!(expired.is_expired());
    
    Ok(())
}

#[test]
fn test_health_status() -> anyhow::Result<()> {
    let healthy = HealthStatus::Healthy {
        last_check: Utc::now(),
        uptime_seconds: 86400, // 1 day
    };
    
    let unhealthy = HealthStatus::Unhealthy {
        last_check: Utc::now(),
        error: "Connection timeout".to_string(),
    };
    
    let degraded = HealthStatus::Degraded {
        last_check: Utc::now(),
        warnings: vec![
            "High memory usage".to_string(),
            "Slow response time".to_string(),
        ],
    };
    
    assert!(matches!(healthy, HealthStatus::Healthy { .. }));
    assert!(matches!(unhealthy, HealthStatus::Unhealthy { .. }));
    
    if let HealthStatus::Degraded { warnings, .. } = degraded {
        assert_eq!(warnings.len(), 2);
    }
    
    Ok(())
}

#[test]
fn test_resource_bundle() -> anyhow::Result<()> {
    let bundle = ResourceBundle {
        cpu_cores: 2.0,
        memory_mb: 4096,
        storage_gb: 100,
        bandwidth_mbps: 100,
    };
    
    assert_eq!(bundle.cpu_cores, 2.0);
    assert_eq!(bundle.memory_mb, 4096);
    assert_eq!(bundle.storage_gb, 100);
    assert_eq!(bundle.bandwidth_mbps, 100);
    
    Ok(())
}

#[test]
fn test_allocate_slot_request() -> anyhow::Result<()> {
    let request = AllocateSlotRequest {
        federation_id: "fed_456".to_string(),
        guardian_npub: "npub1guardian".to_string(),
        subscription_proof: SubscriptionProof {
            subscription_id: Uuid::new_v4(),
            owner_npub: "npub1owner".to_string(),
            provider_npub: "npub1provider".to_string(),
            valid_until: Utc::now() + Duration::days(30),
            slots_allocated: 1,
            signature: "sig123".to_string(),
        },
        resource_requirements: ResourceBundle {
            cpu_cores: 1.0,
            memory_mb: 2048,
            storage_gb: 50,
            bandwidth_mbps: 50,
        },
    };
    
    assert_eq!(request.federation_id, "fed_456");
    assert_eq!(request.guardian_npub, "npub1guardian");
    assert_eq!(request.subscription_proof.slots_allocated, 1);
    
    Ok(())
}

#[test]
fn test_slot_endpoint() -> anyhow::Result<()> {
    let endpoint = SlotEndpoint {
        slot_id: Uuid::new_v4(),
        api_url: Url::parse("http://localhost:8173/api")?,
        p2p_url: Url::parse("ws://localhost:8174")?,
        tor_address: Some("xyz.onion".to_string()),
        access_token: "token123".to_string(),
    };
    
    assert_eq!(endpoint.api_url.port(), Some(8173));
    assert_eq!(endpoint.p2p_url.scheme(), "ws");
    assert!(endpoint.tor_address.is_some());
    
    Ok(())
}

#[test]
fn test_slot_status() -> anyhow::Result<()> {
    let status = SlotStatus {
        slot_id: Uuid::new_v4(),
        state: SlotState::Running {
            federation_id: "fed_789".to_string(),
            endpoint: Url::parse("http://localhost:8173")?,
            health_status: HealthStatus::Healthy {
                last_check: Utc::now(),
                uptime_seconds: 7200,
            },
        },
        resource_usage: ResourceUsage {
            timestamp: Utc::now(),
            cpu_percent: 45.5,
            memory_mb_used: 1024,
            storage_gb_used: 25.5,
            bandwidth_mbps_current: 10.0,
            requests_per_second: Some(100.0),
        },
        federation_info: Some("Test Federation".to_string()),
    };
    
    if let SlotState::Running { .. } = status.state {
        assert_eq!(status.resource_usage.cpu_percent, 45.5);
        assert_eq!(status.resource_usage.memory_mb_used, 1024);
    }
    
    Ok(())
}

#[test]
fn test_slot_serialization() -> anyhow::Result<()> {
    let slot = FedimintSlot {
        id: Uuid::new_v4(),
        owner_npub: "npub1owner".to_string(),
        subscription_id: Uuid::new_v4(),
        state: SlotState::Running {
            federation_id: "fed_abc".to_string(),
            endpoint: Url::parse("http://localhost:8173")?,
            health_status: HealthStatus::Healthy {
                last_check: Utc::now(),
                uptime_seconds: 3600,
            },
        },
        federation_id: Some("fed_abc".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    let json = serde_json::to_string(&slot)?;
    let decoded: FedimintSlot = serde_json::from_str(&json)?;
    
    assert_eq!(decoded.id, slot.id);
    assert_eq!(decoded.owner_npub, slot.owner_npub);
    assert_eq!(decoded.federation_id, slot.federation_id);
    
    Ok(())
}