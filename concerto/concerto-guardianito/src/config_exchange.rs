use concerto_common::*;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::borrow::Cow;
use tracing::{info, warn};
use ::url::Url;

/// Configuration Exchange Protocol
/// Allows guardians to share and synchronize federation configuration
pub struct ConfigExchange {
    federation_id: String,
    our_config: GuardianConfig,
    peer_configs: HashMap<String, GuardianConfig>,
    nostr_client: crate::nostr_client::NostrClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianConfig {
    pub guardian_npub: String,
    pub federation_id: String,
    pub fedimint_port: u16,
    pub api_url: Url,
    pub p2p_url: Url,
    pub slot_allocation: SlotAllocation,
    pub provider_info: ProviderInfo,
    pub consensus_params: ConsensusParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub provider_npub: String,
    pub provider_url: Url,
    pub slot_endpoint: Url,
    pub allocated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusParams {
    pub threshold: usize,
    pub total_peers: usize,
    pub block_time_ms: u64,
    pub epoch_length: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigExchangeEvent {
    pub federation_id: String,
    pub event_type: ConfigEventType,
    pub payload: String, // Encrypted JSON
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigEventType {
    Request,
    Response,
    Update,
    Sync,
    Verify,
}

impl ConfigExchange {
    pub fn new(
        federation_id: String,
        our_config: GuardianConfig,
        nostr_client: crate::nostr_client::NostrClient,
    ) -> Self {
        Self {
            federation_id,
            our_config,
            peer_configs: HashMap::new(),
            nostr_client,
        }
    }

    /// Request configuration from all guardians
    pub async fn request_configs(&self) -> anyhow::Result<()> {
        info!("Requesting configurations from federation guardians");
        
        let request = ConfigExchangeEvent {
            federation_id: self.federation_id.clone(),
            event_type: ConfigEventType::Request,
            payload: serde_json::to_string(&ConfigRequest {
                from: self.our_config.guardian_npub.clone(),
                federation_id: self.federation_id.clone(),
                timestamp: chrono::Utc::now(),
            })?,
            signature: self.sign_payload("")?, // Sign with our key
        };

        self.broadcast_config_event(request).await?;
        Ok(())
    }

    /// Share our configuration with peers
    pub async fn share_config(&self) -> anyhow::Result<()> {
        info!("Sharing our configuration with federation guardians");
        
        let response = ConfigExchangeEvent {
            federation_id: self.federation_id.clone(),
            event_type: ConfigEventType::Response,
            payload: self.encrypt_config(&self.our_config)?,
            signature: self.sign_payload(&serde_json::to_string(&self.our_config)?)?,
        };

        self.broadcast_config_event(response).await?;
        Ok(())
    }

    /// Process incoming configuration event
    pub async fn process_config_event(&mut self, event: Event) -> anyhow::Result<()> {
        let config_event: ConfigExchangeEvent = serde_json::from_str(&event.content)?;
        
        // Verify signature
        if !self.verify_signature(&config_event)? {
            warn!("Invalid signature on config event");
            return Ok(());
        }

        match config_event.event_type {
            ConfigEventType::Request => {
                // Respond with our configuration
                self.share_config().await?;
            }
            ConfigEventType::Response => {
                // Store peer configuration
                let config = self.decrypt_config(&config_event.payload)?;
                self.peer_configs.insert(config.guardian_npub.clone(), config);
                info!("Received configuration from guardian");
            }
            ConfigEventType::Update => {
                // Update peer configuration
                let config = self.decrypt_config(&config_event.payload)?;
                self.peer_configs.insert(config.guardian_npub.clone(), config);
                info!("Updated configuration for guardian");
            }
            ConfigEventType::Sync => {
                // Full sync request
                self.handle_sync_request().await?;
            }
            ConfigEventType::Verify => {
                // Verification request
                self.handle_verification().await?;
            }
        }

        Ok(())
    }

    /// Synchronize configurations across all guardians
    pub async fn sync_configs(&mut self) -> anyhow::Result<()> {
        info!("Synchronizing configurations with {} guardians", self.peer_configs.len());
        
        // Request updates from all guardians
        self.request_configs().await?;
        
        // Wait for responses (in real implementation, would use timeout)
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        
        // Verify consistency
        self.verify_config_consistency()?;
        
        Ok(())
    }

    /// Generate fedimint configuration file
    pub fn generate_fedimint_config(&self) -> anyhow::Result<FedimintConfig> {
        let mut guardians = vec![self.our_config.to_fedimint_peer()];
        
        for config in self.peer_configs.values() {
            guardians.push(config.to_fedimint_peer());
        }

        Ok(FedimintConfig {
            federation_id: self.federation_id.clone(),
            guardians,
            consensus: FedimintConsensus {
                threshold: self.our_config.consensus_params.threshold,
                block_time_ms: self.our_config.consensus_params.block_time_ms,
                epoch_length: self.our_config.consensus_params.epoch_length,
            },
            modules: self.generate_module_configs()?,
        })
    }

    /// Verify configuration consistency across guardians
    fn verify_config_consistency(&self) -> anyhow::Result<bool> {
        // Check that all guardians have same federation_id
        for config in self.peer_configs.values() {
            if config.federation_id != self.federation_id {
                warn!("Federation ID mismatch with guardian {}", config.guardian_npub);
                return Ok(false);
            }
            
            // Check consensus parameters match
            if config.consensus_params.threshold != self.our_config.consensus_params.threshold {
                warn!("Consensus threshold mismatch with guardian {}", config.guardian_npub);
                return Ok(false);
            }
        }

        info!("Configuration consistency verified");
        Ok(true)
    }

    async fn broadcast_config_event(&self, event: ConfigExchangeEvent) -> anyhow::Result<()> {
        let unsigned = EventBuilder::new(
            Kind::from(30505), // Custom kind for config exchange
            serde_json::to_string(&event)?,
        )
            .tags(vec![
                Tag::custom(TagKind::Custom(Cow::Borrowed("federation")), vec![self.federation_id.clone()]),
                Tag::custom(TagKind::Custom(Cow::Borrowed("type")), vec!["config_exchange".to_string()]),
            ])
            .build(self.nostr_client.keys.public_key());
        let nostr_event = unsigned.sign_with_keys(&self.nostr_client.keys)?;

        self.nostr_client.client.send_event(&nostr_event).await?;
        Ok(())
    }

    async fn handle_sync_request(&self) -> anyhow::Result<()> {
        // Send full configuration set
        self.share_config().await?;
        Ok(())
    }

    async fn handle_verification(&self) -> anyhow::Result<()> {
        // Send verification response
        let verification = ConfigVerification {
            guardian_npub: self.our_config.guardian_npub.clone(),
            federation_id: self.federation_id.clone(),
            config_hash: self.calculate_config_hash()?,
            timestamp: chrono::Utc::now(),
        };

        let event = ConfigExchangeEvent {
            federation_id: self.federation_id.clone(),
            event_type: ConfigEventType::Verify,
            payload: serde_json::to_string(&verification)?,
            signature: self.sign_payload(&serde_json::to_string(&verification)?)?,
        };

        self.broadcast_config_event(event).await?;
        Ok(())
    }

    fn encrypt_config(&self, config: &GuardianConfig) -> anyhow::Result<String> {
        // In real implementation, encrypt with federation shared key
        Ok(serde_json::to_string(config)?)
    }

    fn decrypt_config(&self, encrypted: &str) -> anyhow::Result<GuardianConfig> {
        // In real implementation, decrypt with federation shared key
        Ok(serde_json::from_str(encrypted)?)
    }

    fn sign_payload(&self, payload: &str) -> anyhow::Result<String> {
        // In real implementation, sign with our Nostr key
        Ok(format!("sig_{}", self.our_config.guardian_npub))
    }

    fn verify_signature(&self, event: &ConfigExchangeEvent) -> anyhow::Result<bool> {
        // In real implementation, verify with sender's public key
        Ok(true)
    }

    fn calculate_config_hash(&self) -> anyhow::Result<String> {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_string(&self.our_config)?);
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn generate_module_configs(&self) -> anyhow::Result<HashMap<String, serde_json::Value>> {
        let mut modules = HashMap::new();
        
        // Lightning module
        modules.insert("lightning".to_string(), serde_json::json!({
            "network": "bitcoin",
            "finality_delay": 10,
        }));
        
        // Mint module
        modules.insert("mint".to_string(), serde_json::json!({
            "fee_consensus": 1000,
            "peer_punishment": true,
        }));
        
        // Wallet module
        modules.insert("wallet".to_string(), serde_json::json!({
            "network": "bitcoin",
            "finality_delay": 10,
            "fee_rate": 1,
        }));
        
        Ok(modules)
    }
}

impl GuardianConfig {
    fn to_fedimint_peer(&self) -> FedimintPeer {
        FedimintPeer {
            name: format!("guardian_{}", self.guardian_npub),
            api_url: self.api_url.clone(),
            p2p_url: self.p2p_url.clone(),
            public_key: self.guardian_npub.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigRequest {
    from: String,
    federation_id: String,
    timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigVerification {
    guardian_npub: String,
    federation_id: String,
    config_hash: String,
    timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FedimintConfig {
    pub federation_id: String,
    pub guardians: Vec<FedimintPeer>,
    pub consensus: FedimintConsensus,
    pub modules: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FedimintPeer {
    pub name: String,
    pub api_url: Url,
    pub p2p_url: Url,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FedimintConsensus {
    pub threshold: usize,
    pub block_time_ms: u64,
    pub epoch_length: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_serialization() -> anyhow::Result<()> {
        let config = GuardianConfig {
            guardian_npub: "npub1test".to_string(),
            federation_id: "fed123".to_string(),
            fedimint_port: 8173,
            api_url: "http://localhost:8173".parse()?,
            p2p_url: "ws://localhost:8174".parse()?,
            slot_allocation: SlotAllocation {
                slot_id: uuid::Uuid::new_v4(),
                provider_npub: "npub1provider".to_string(),
                allocated_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::days(30),
            },
            provider_info: ProviderInfo {
                provider_npub: "npub1provider".to_string(),
                provider_url: "https://provider.example".parse()?,
                slot_endpoint: "https://provider.example/slot".parse()?,
                allocated_at: chrono::Utc::now(),
            },
            consensus_params: ConsensusParams {
                threshold: 2,
                total_peers: 3,
                block_time_ms: 1000,
                epoch_length: 100,
            },
        };

        let json = serde_json::to_string(&config)?;
        let decoded: GuardianConfig = serde_json::from_str(&json)?;
        
        assert_eq!(decoded.federation_id, config.federation_id);
        assert_eq!(decoded.fedimint_port, config.fedimint_port);
        
        Ok(())
    }
}