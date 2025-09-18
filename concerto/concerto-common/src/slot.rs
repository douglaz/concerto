use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

/// Represents a fedimint slot that can be allocated to a federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FedimintSlot {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub state: SlotState,
    pub allocation: Option<SlotAllocation>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SlotState {
    /// Slot is available for allocation
    Available,
    /// Slot has been allocated to a federation
    Allocated { federation_id: String },
    /// Slot is in the process of being launched
    Launching { federation_id: String },
    /// Slot is running and operational
    Running { 
        federation_id: String,
        endpoint: Url,
        health_status: HealthStatus,
    },
    /// Slot has been stopped
    Stopped { reason: String },
    /// Slot has encountered an error
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotAllocation {
    pub federation_id: String,
    pub guardian_npub: String,
    pub provider_url: Url,
    pub allocated_at: DateTime<Utc>,
    pub resources: ResourceBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded { issues: Vec<String> },
    Unhealthy { reason: String },
    Unknown,
}

/// Resources allocated to a slot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBundle {
    pub cpu_cores: f32,
    pub memory_gb: f32,
    pub storage_gb: f32,
    pub bandwidth_mbps: Option<f32>,
}

/// Request to allocate a slot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocateSlotRequest {
    pub slot_id: Uuid,
    pub federation_id: String,
    pub guardian_npub: String,
    pub subscription_proof: crate::SubscriptionProof,
    pub requested_resources: Option<ResourceBundle>,
}

/// Response from slot allocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotEndpoint {
    pub slot_id: Uuid,
    pub endpoint: Url,
    pub api_key: Option<String>,
    pub resources: ResourceBundle,
}

/// Slot status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotStatus {
    pub slot_id: Uuid,
    pub state: SlotState,
    pub uptime_seconds: Option<u64>,
    pub last_health_check: Option<DateTime<Utc>>,
    pub resource_usage: Option<ResourceUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_percent: f32,
    pub memory_used_gb: f32,
    pub storage_used_gb: f32,
    pub bandwidth_used_mbps: f32,
}

impl FedimintSlot {
    pub fn new(subscription_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            subscription_id,
            state: SlotState::Available,
            allocation: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn allocate(&mut self, allocation: SlotAllocation) {
        self.state = SlotState::Allocated {
            federation_id: allocation.federation_id.clone(),
        };
        self.allocation = Some(allocation);
        self.updated_at = Utc::now();
    }

    pub fn launch(&mut self, federation_id: String) {
        self.state = SlotState::Launching { federation_id };
        self.updated_at = Utc::now();
    }

    pub fn set_running(&mut self, federation_id: String, endpoint: Url) {
        self.state = SlotState::Running {
            federation_id,
            endpoint,
            health_status: HealthStatus::Healthy,
        };
        self.updated_at = Utc::now();
    }

    pub fn stop(&mut self, reason: String) {
        self.state = SlotState::Stopped { reason };
        self.updated_at = Utc::now();
    }

    pub fn is_available(&self) -> bool {
        matches!(self.state, SlotState::Available)
    }

    pub fn is_allocated(&self) -> bool {
        !self.is_available()
    }
}

impl ResourceBundle {
    pub fn standard() -> Self {
        Self {
            cpu_cores: 0.5,
            memory_gb: 1.0,
            storage_gb: 10.0,
            bandwidth_mbps: Some(100.0),
        }
    }

    pub fn premium() -> Self {
        Self {
            cpu_cores: 2.0,
            memory_gb: 4.0,
            storage_gb: 50.0,
            bandwidth_mbps: Some(1000.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_lifecycle() {
        let subscription_id = Uuid::new_v4();
        let mut slot = FedimintSlot::new(subscription_id);
        
        assert!(slot.is_available());
        assert!(!slot.is_allocated());

        let allocation = SlotAllocation {
            federation_id: "fed123".to_string(),
            guardian_npub: "npub1test".to_string(),
            provider_url: Url::parse("https://provider.example.com").unwrap(),
            allocated_at: Utc::now(),
            resources: ResourceBundle::standard(),
        };

        slot.allocate(allocation);
        assert!(!slot.is_available());
        assert!(slot.is_allocated());

        slot.launch("fed123".to_string());
        assert!(matches!(slot.state, SlotState::Launching { .. }));

        let endpoint = Url::parse("https://guardian.example.com:8080").unwrap();
        slot.set_running("fed123".to_string(), endpoint);
        assert!(matches!(slot.state, SlotState::Running { .. }));

        slot.stop("Maintenance".to_string());
        assert!(matches!(slot.state, SlotState::Stopped { .. }));
    }

    #[test]
    fn test_resource_bundles() {
        let standard = ResourceBundle::standard();
        assert_eq!(standard.cpu_cores, 0.5);
        assert_eq!(standard.memory_gb, 1.0);

        let premium = ResourceBundle::premium();
        assert_eq!(premium.cpu_cores, 2.0);
        assert_eq!(premium.memory_gb, 4.0);
    }
}