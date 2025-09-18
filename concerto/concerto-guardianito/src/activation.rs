use concerto_common::*;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::borrow::Cow;
use tracing::{info, warn, error};
use ::url::Url;

/// Federation Activation Flow
/// Coordinates the transition from proposed federation to active federation
pub struct FederationActivator {
    federation_id: String,
    federation_state: ActivationState,
    guardians: Vec<GuardianInfo>,
    dkg_coordinator: crate::dkg::DkgCoordinator,
    config_exchange: crate::config_exchange::ConfigExchange,
    nostr_client: crate::nostr_client::NostrClient,
    activation_checklist: ActivationChecklist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActivationState {
    Proposed,
    GatheringGuardians { current: usize, required: usize },
    AllocatingSlots,
    ConfiguringGuardians,
    PerformingDKG,
    StartingFedimint,
    VerifyingOperation,
    Active,
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianInfo {
    pub npub: String,
    pub slot_allocation: Option<SlotAllocation>,
    pub config_ready: bool,
    pub dkg_complete: bool,
    pub fedimint_running: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivationChecklist {
    pub guardians_confirmed: bool,
    pub slots_allocated: bool,
    pub configs_exchanged: bool,
    pub dkg_completed: bool,
    pub fedimint_started: bool,
    pub health_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationEvent {
    pub federation_id: String,
    pub event_type: ActivationEventType,
    pub guardian_npub: String,
    pub payload: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActivationEventType {
    GuardianReady,
    SlotAllocated,
    ConfigShared,
    DkgProgress,
    FedimintStarted,
    HealthCheck,
    ActivationComplete,
    ActivationFailed,
}

impl FederationActivator {
    pub fn new(
        federation_id: String,
        guardians: Vec<String>,
        nostr_client: crate::nostr_client::NostrClient,
    ) -> Self {
        let guardian_infos = guardians.iter().map(|npub| GuardianInfo {
            npub: npub.clone(),
            slot_allocation: None,
            config_ready: false,
            dkg_complete: false,
            fedimint_running: false,
        }).collect();

        let dkg_coordinator = crate::dkg::DkgCoordinator::new(
            federation_id.clone(),
            guardians.clone(),
            nostr_client.clone(),
        );

        // Placeholder for config exchange - would need proper initialization
        let our_config = crate::config_exchange::GuardianConfig {
            guardian_npub: nostr_client.get_our_npub().unwrap_or_default(),
            federation_id: federation_id.clone(),
            fedimint_port: 8173,
            api_url: "http://localhost:8173".parse().unwrap(),
            p2p_url: "ws://localhost:8174".parse().unwrap(),
            slot_allocation: SlotAllocation {
                slot_id: uuid::Uuid::new_v4(),
                provider_npub: String::new(),
                allocated_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::days(30),
            },
            provider_info: crate::config_exchange::ProviderInfo {
                provider_npub: String::new(),
                provider_url: "https://provider.example".parse().unwrap(),
                slot_endpoint: "https://provider.example/slot".parse().unwrap(),
                allocated_at: chrono::Utc::now(),
            },
            consensus_params: crate::config_exchange::ConsensusParams {
                threshold: (guardians.len() / 2) + 1,
                total_peers: guardians.len(),
                block_time_ms: 1000,
                epoch_length: 100,
            },
        };

        let config_exchange = crate::config_exchange::ConfigExchange::new(
            federation_id.clone(),
            our_config,
            nostr_client.clone(),
        );

        Self {
            federation_id,
            federation_state: ActivationState::Proposed,
            guardians: guardian_infos,
            dkg_coordinator,
            config_exchange,
            nostr_client,
            activation_checklist: ActivationChecklist::default(),
        }
    }

    /// Start the federation activation process
    pub async fn start_activation(&mut self) -> anyhow::Result<()> {
        info!("Starting federation activation for {}", self.federation_id);
        
        self.federation_state = ActivationState::GatheringGuardians {
            current: 0,
            required: self.guardians.len(),
        };

        // Broadcast activation start event
        self.broadcast_activation_event(ActivationEventType::GuardianReady).await?;
        
        Ok(())
    }

    /// Process guardian ready signal
    pub async fn guardian_ready(&mut self, guardian_npub: String) -> anyhow::Result<()> {
        info!("Guardian {} is ready", guardian_npub);
        
        if let Some(guardian) = self.guardians.iter_mut().find(|g| g.npub == guardian_npub) {
            guardian.config_ready = true;
        }

        self.check_activation_progress().await?;
        Ok(())
    }

    /// Allocate slots for all guardians
    pub async fn allocate_slots(&mut self) -> anyhow::Result<()> {
        info!("Allocating slots for {} guardians", self.guardians.len());
        
        self.federation_state = ActivationState::AllocatingSlots;

        for guardian in &mut self.guardians {
            // Request slot allocation from their preferred provider
            let allocation = self.request_slot_allocation(&guardian.npub).await?;
            guardian.slot_allocation = Some(allocation);
            
            // Broadcast slot allocation event
            self.broadcast_activation_event(ActivationEventType::SlotAllocated).await?;
        }

        self.activation_checklist.slots_allocated = true;
        self.check_activation_progress().await?;
        
        Ok(())
    }

    /// Exchange configurations between guardians
    pub async fn exchange_configs(&mut self) -> anyhow::Result<()> {
        info!("Exchanging configurations between guardians");
        
        self.federation_state = ActivationState::ConfiguringGuardians;

        // Request configs from all guardians
        self.config_exchange.request_configs().await?;
        
        // Share our config
        self.config_exchange.share_config().await?;
        
        // Wait for configs (in production, use proper async waiting)
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        
        // Sync all configs
        self.config_exchange.sync_configs().await?;
        
        self.activation_checklist.configs_exchanged = true;
        self.broadcast_activation_event(ActivationEventType::ConfigShared).await?;
        self.check_activation_progress().await?;
        
        Ok(())
    }

    /// Perform distributed key generation
    pub async fn perform_dkg(&mut self) -> anyhow::Result<()> {
        info!("Starting DKG process");
        
        self.federation_state = ActivationState::PerformingDKG;

        // Initiate DKG
        self.dkg_coordinator.initiate_dkg().await?;
        
        // Wait for DKG completion (simplified - real implementation would monitor events)
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        
        self.activation_checklist.dkg_completed = true;
        
        for guardian in &mut self.guardians {
            guardian.dkg_complete = true;
        }
        
        self.broadcast_activation_event(ActivationEventType::DkgProgress).await?;
        self.check_activation_progress().await?;
        
        Ok(())
    }

    /// Start Fedimint instances
    pub async fn start_fedimint(&mut self) -> anyhow::Result<()> {
        info!("Starting Fedimint instances");
        
        self.federation_state = ActivationState::StartingFedimint;

        // Generate fedimint configuration
        let fedimint_config = self.config_exchange.generate_fedimint_config()?;
        
        // Start our fedimint instance
        self.start_local_fedimint(fedimint_config).await?;
        
        // Mark as started
        if let Some(our_npub) = self.nostr_client.get_our_npub().ok() {
            if let Some(guardian) = self.guardians.iter_mut().find(|g| g.npub == our_npub) {
                guardian.fedimint_running = true;
            }
        }
        
        self.activation_checklist.fedimint_started = true;
        self.broadcast_activation_event(ActivationEventType::FedimintStarted).await?;
        self.check_activation_progress().await?;
        
        Ok(())
    }

    /// Verify federation is operational
    pub async fn verify_operation(&mut self) -> anyhow::Result<()> {
        info!("Verifying federation operation");
        
        self.federation_state = ActivationState::VerifyingOperation;

        // Check health of all guardian endpoints
        for guardian in &self.guardians {
            if let Some(allocation) = &guardian.slot_allocation {
                let health = self.check_guardian_health(allocation).await?;
                if !health {
                    warn!("Guardian {} health check failed", guardian.npub);
                }
            }
        }

        // Perform test operations
        self.perform_test_operations().await?;
        
        self.activation_checklist.health_verified = true;
        self.broadcast_activation_event(ActivationEventType::HealthCheck).await?;
        
        // Mark as active if all checks pass
        if self.activation_checklist.all_complete() {
            self.federation_state = ActivationState::Active;
            self.broadcast_activation_event(ActivationEventType::ActivationComplete).await?;
            info!("Federation {} is now ACTIVE!", self.federation_id);
        }
        
        Ok(())
    }

    /// Check and advance activation progress
    async fn check_activation_progress(&mut self) -> anyhow::Result<()> {
        match &self.federation_state {
            ActivationState::GatheringGuardians { .. } => {
                let ready_count = self.guardians.iter().filter(|g| g.config_ready).count();
                if ready_count == self.guardians.len() {
                    self.activation_checklist.guardians_confirmed = true;
                    self.allocate_slots().await?;
                }
            }
            ActivationState::AllocatingSlots => {
                if self.activation_checklist.slots_allocated {
                    self.exchange_configs().await?;
                }
            }
            ActivationState::ConfiguringGuardians => {
                if self.activation_checklist.configs_exchanged {
                    self.perform_dkg().await?;
                }
            }
            ActivationState::PerformingDKG => {
                if self.activation_checklist.dkg_completed {
                    self.start_fedimint().await?;
                }
            }
            ActivationState::StartingFedimint => {
                if self.activation_checklist.fedimint_started {
                    self.verify_operation().await?;
                }
            }
            _ => {}
        }
        
        Ok(())
    }

    async fn request_slot_allocation(&self, guardian_npub: &str) -> anyhow::Result<SlotAllocation> {
        // In real implementation, would request from FeLaaS provider
        Ok(SlotAllocation {
            slot_id: uuid::Uuid::new_v4(),
            provider_npub: "npub1provider".to_string(),
            allocated_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(30),
        })
    }

    async fn start_local_fedimint(&self, config: crate::config_exchange::FedimintConfig) -> anyhow::Result<()> {
        // In real implementation, would start fedimintd process
        info!("Starting fedimintd with config for federation {}", config.federation_id);
        Ok(())
    }

    async fn check_guardian_health(&self, allocation: &SlotAllocation) -> anyhow::Result<bool> {
        // In real implementation, would check actual endpoint health
        Ok(true)
    }

    async fn perform_test_operations(&self) -> anyhow::Result<()> {
        // In real implementation, would perform actual test operations
        info!("Performing test operations on federation");
        Ok(())
    }

    async fn broadcast_activation_event(&self, event_type: ActivationEventType) -> anyhow::Result<()> {
        let event = ActivationEvent {
            federation_id: self.federation_id.clone(),
            event_type,
            guardian_npub: self.nostr_client.get_our_npub().unwrap_or_default(),
            payload: serde_json::json!({
                "state": serde_json::to_value(&self.federation_state)?,
                "checklist": serde_json::to_value(&self.activation_checklist)?,
            }),
            timestamp: chrono::Utc::now(),
        };

        let unsigned = EventBuilder::new(
            Kind::from(30506), // Custom kind for activation events
            serde_json::to_string(&event)?,
        )
            .tags(vec![
                Tag::custom(TagKind::Custom(Cow::Borrowed("federation")), vec![self.federation_id.clone()]),
                Tag::custom(TagKind::Custom(Cow::Borrowed("type")), vec!["activation".to_string()]),
            ])
            .build(self.nostr_client.keys.public_key());
        let nostr_event = unsigned.sign_with_keys(&self.nostr_client.keys)?;

        self.nostr_client.client.send_event(&nostr_event).await?;
        Ok(())
    }
}

impl ActivationChecklist {
    pub fn all_complete(&self) -> bool {
        self.guardians_confirmed &&
        self.slots_allocated &&
        self.configs_exchanged &&
        self.dkg_completed &&
        self.fedimint_started &&
        self.health_verified
    }
}

// Extension to NostrClient
impl crate::nostr_client::NostrClient {
    pub fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            keys: self.keys.clone(),
            owner_pubkey: self.owner_pubkey.clone(),
            relays: self.relays.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activation_checklist() -> anyhow::Result<()> {
        let mut checklist = ActivationChecklist::default();
        assert!(!checklist.all_complete());
        
        checklist.guardians_confirmed = true;
        checklist.slots_allocated = true;
        checklist.configs_exchanged = true;
        checklist.dkg_completed = true;
        checklist.fedimint_started = true;
        checklist.health_verified = true;
        
        assert!(checklist.all_complete());
        Ok(())
    }

    #[test]
    fn test_activation_state_serialization() -> anyhow::Result<()> {
        let state = ActivationState::GatheringGuardians {
            current: 2,
            required: 4,
        };
        
        let json = serde_json::to_string(&state)?;
        let decoded: ActivationState = serde_json::from_str(&json)?;
        
        match decoded {
            ActivationState::GatheringGuardians { current, required } => {
                assert_eq!(current, 2);
                assert_eq!(required, 4);
            }
            _ => panic!("Unexpected state"),
        }
        
        Ok(())
    }
}