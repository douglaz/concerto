use concerto_common::*;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn, error};
// Fedimint integration will be handled by the actual Fedimint nodes
// This coordinator just handles Nostr-based coordination of setup codes

/// DKG Coordinator for Nostr-based setup code exchange
/// The actual DKG is performed by Fedimint nodes
pub struct DkgCoordinator {
    federation_id: String,
    federation_name: String,
    participant_npubs: Vec<String>,
    setup_codes: HashMap<String, String>, // npub -> setup_code
    nostr_client: crate::nostr_client::NostrClient,
    state: DkgState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DkgState {
    NotStarted,
    WaitingForSetupCodes { received: usize, expected: usize },
    AllCodesReceived,
    Completed { message: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DkgSetupCode {
    from_npub: String,
    peer_id: u16,
    setup_code: String,
    api_url: String,
}

impl DkgCoordinator {
    pub fn new(
        federation_id: String,
        federation_name: String,
        participant_npubs: Vec<String>,
        nostr_client: crate::nostr_client::NostrClient,
    ) -> Self {
        Self {
            federation_id,
            federation_name,
            participant_npubs: participant_npubs.clone(),
            setup_codes: HashMap::new(),
            nostr_client,
            state: DkgState::NotStarted,
        }
    }

    /// Start collecting setup codes
    pub async fn start_collection(&mut self) -> anyhow::Result<()> {
        info!("Starting setup code collection for federation {}", self.federation_id);
        
        self.state = DkgState::WaitingForSetupCodes {
            received: 0,
            expected: self.participant_npubs.len(),
        };
        
        // Broadcast that we're ready to collect setup codes
        let init_event = DkgCoordinationEvent {
            federation_id: self.federation_id.clone(),
            round: 0,
            message_type: DkgMessageType::Initiate {
                participants: self.participant_npubs.clone(),
            },
            payload: serde_json::to_string(&self.federation_name)?,
        };
        
        self.nostr_client.publish_dkg_coordination(init_event).await?;
        
        Ok(())
    }

    /// Process incoming DKG setup code from another guardian
    pub async fn process_setup_code(&mut self, setup_code: DkgSetupCode) -> anyhow::Result<()> {
        info!("Received setup code from {}", setup_code.from_npub);
        
        // Store the setup code
        self.setup_codes.insert(setup_code.from_npub.clone(), setup_code.setup_code.clone());
        
        // Update state
        if let DkgState::WaitingForSetupCodes { received, expected } = &mut self.state {
            *received += 1;
            if *received == *expected {
                self.state = DkgState::AllCodesReceived;
                info!("All setup codes received for federation {}", self.federation_id);
            }
        }
        
        Ok(())
    }
    
    /// Broadcast our setup code to other guardians via Nostr
    pub async fn broadcast_setup_code(&mut self, our_setup_code: String) -> anyhow::Result<()> {
        let setup_msg = DkgSetupCode {
            from_npub: self.nostr_client.get_our_npub()?,
            peer_id: 0, // Will be set by Fedimint
            setup_code: our_setup_code.clone(),
            api_url: String::new(), // Guardians know their own API URLs
        };
        
        let event = DkgCoordinationEvent {
            federation_id: self.federation_id.clone(),
            round: 0,
            message_type: DkgMessageType::SetupCode,
            payload: serde_json::to_string(&setup_msg)?,
        };
        
        self.nostr_client.publish_dkg_coordination(event).await?;
        Ok(())
    }

    /// Check if all setup codes have been collected
    pub fn all_codes_collected(&self) -> bool {
        self.setup_codes.len() == self.participant_npubs.len()
    }
    
    /// Get collected setup codes
    pub fn get_setup_codes(&self) -> Vec<String> {
        self.setup_codes.values().cloned().collect()
    }
}

fn calculate_threshold(num_participants: usize) -> usize {
    // Standard threshold: n/2 + 1
    (num_participants / 2) + 1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DkgInitMessage {
    federation_id: String,
    threshold: usize,
    participants: Vec<String>,
}

// Extension to NostrClient for DKG operations
impl crate::nostr_client::NostrClient {
    pub fn get_our_npub(&self) -> anyhow::Result<String> {
        Ok(self.keys.public_key().to_bech32()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_threshold() -> anyhow::Result<()> {
        assert_eq!(calculate_threshold(3), 2);
        assert_eq!(calculate_threshold(4), 3);
        assert_eq!(calculate_threshold(5), 3);
        assert_eq!(calculate_threshold(7), 4);
        Ok(())
    }

    #[test]
    fn test_dkg_state_transitions() -> anyhow::Result<()> {
        let state = DkgState::NotStarted;
        
        // Test serialization
        let json = serde_json::to_string(&state)?;
        let decoded: DkgState = serde_json::from_str(&json)?;
        
        match decoded {
            DkgState::NotStarted => {},
            _ => panic!("Unexpected state"),
        }
        
        Ok(())
    }
}