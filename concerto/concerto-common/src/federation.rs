use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

/// Represents a fedimint federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Federation {
    /// Deterministic ID from proposal event
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub initiator_npub: String,
    pub status: FederationStatus,
    pub guardian_slots: Vec<GuardianSlot>,
    pub consensus_config: ConsensusConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianSlot {
    pub guardian_npub: String,
    pub slot_count: u32,
    pub status: GuardianStatus,
    pub endpoints: Vec<Url>,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GuardianStatus {
    /// Applied but not yet approved
    Pending,
    /// Approved to join federation
    Approved,
    /// Rejected application
    Rejected { reason: String },
    /// Active guardian with running slots
    Active,
    /// Guardian has left the federation
    Left { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FederationStatus {
    /// Federation has been proposed
    Proposed { 
        total_slots: u32,
        open_slots: u32,
    },
    /// Federation is forming (accepting guardians)
    Forming { 
        filled_slots: u32,
        total_slots: u32,
    },
    /// All slots filled, configuring guardians
    Configuring,
    /// Running distributed key generation
    RunningDkg { 
        round: u32,
        participants: Vec<String>,
    },
    /// Federation is active and operational
    Active {
        epoch: u64,
        last_consensus: DateTime<Utc>,
    },
    /// Federation is inactive
    Inactive { 
        reason: String,
        since: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    pub threshold: u32,
    pub total_guardians: u32,
    pub block_time_ms: u64,
    pub consensus_params: HashMap<String, String>,
}

/// Participation of a guardian in a federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationParticipation {
    pub federation_id: String,
    pub my_role: ParticipantRole,
    pub my_slots: Vec<uuid::Uuid>,
    pub other_guardians: Vec<GuardianInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParticipantRole {
    /// Started the federation proposal
    Initiator,
    /// Approved participant
    Guardian,
    /// Waiting for approval
    Candidate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianInfo {
    pub npub: String,
    pub nickname: Option<String>,
    pub slots: u32,
    pub endpoints: Vec<Url>,
    pub status: GuardianStatus,
}

/// Requirements for joining a federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationRequirements {
    pub min_slots: u32,
    pub min_subscription_tier: Option<String>,
    pub required_features: Vec<String>,
    pub geographic_diversity: Option<GeographicRequirement>,
    pub custom_requirements: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeographicRequirement {
    pub required_regions: Vec<String>,
    pub max_guardians_per_region: u32,
}

/// Proposal to create a new federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationProposal {
    pub name: String,
    pub description: Option<String>,
    pub initiator_slots: u32,
    pub total_slots: u32,
    pub requirements: FederationRequirements,
    pub consensus_config: ConsensusConfig,
}

/// Application to join a federation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianApplication {
    pub federation_id: String,
    pub applicant_npub: String,
    pub slots_to_contribute: u32,
    pub preferred_providers: Vec<Url>,
    pub message: Option<String>,
}

/// Decision on a guardian application
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationDecision {
    pub application_id: String,
    pub federation_id: String,
    pub applicant_npub: String,
    pub decision: Decision,
    pub decided_by: String,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Decision {
    Approved,
    Rejected { reason: String },
}

impl Federation {
    pub fn from_proposal(id: String, proposal: FederationProposal, initiator_npub: String) -> Self {
        let now = Utc::now();
        
        let initiator_slot = GuardianSlot {
            guardian_npub: initiator_npub.clone(),
            slot_count: proposal.initiator_slots,
            status: GuardianStatus::Approved,
            endpoints: vec![],
            joined_at: now,
        };

        Self {
            id,
            name: proposal.name,
            description: proposal.description,
            initiator_npub,
            status: FederationStatus::Proposed {
                total_slots: proposal.total_slots,
                open_slots: proposal.total_slots - proposal.initiator_slots,
            },
            guardian_slots: vec![initiator_slot],
            consensus_config: proposal.consensus_config,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn add_guardian(&mut self, guardian: GuardianSlot) {
        self.guardian_slots.push(guardian);
        self.update_status();
        self.updated_at = Utc::now();
    }

    pub fn update_status(&mut self) {
        let total_slots: u32 = self.guardian_slots.iter()
            .filter(|g| matches!(g.status, GuardianStatus::Approved | GuardianStatus::Active))
            .map(|g| g.slot_count)
            .sum();

        let total_required = self.consensus_config.total_guardians;

        self.status = if total_slots < total_required {
            FederationStatus::Forming {
                filled_slots: total_slots,
                total_slots: total_required,
            }
        } else if total_slots == total_required {
            FederationStatus::Configuring
        } else {
            self.status.clone()
        };
    }

    pub fn is_accepting_guardians(&self) -> bool {
        matches!(self.status, FederationStatus::Proposed { .. } | FederationStatus::Forming { .. })
    }

    pub fn can_start_dkg(&self) -> bool {
        matches!(self.status, FederationStatus::Configuring)
    }
}

impl ConsensusConfig {
    pub fn standard(total_guardians: u32) -> Self {
        let threshold = (total_guardians * 2 / 3) + 1; // Byzantine fault tolerance
        Self {
            threshold,
            total_guardians,
            block_time_ms: 1000,
            consensus_params: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_federation_from_proposal() {
        let proposal = FederationProposal {
            name: "Test Federation".to_string(),
            description: Some("A test federation".to_string()),
            initiator_slots: 2,
            total_slots: 4,
            requirements: FederationRequirements {
                min_slots: 1,
                min_subscription_tier: None,
                required_features: vec![],
                geographic_diversity: None,
                custom_requirements: HashMap::new(),
            },
            consensus_config: ConsensusConfig::standard(4),
        };

        let fed = Federation::from_proposal(
            "fed123".to_string(),
            proposal,
            "npub1initiator".to_string(),
        );

        assert_eq!(fed.name, "Test Federation");
        assert_eq!(fed.guardian_slots.len(), 1);
        assert!(matches!(fed.status, FederationStatus::Proposed { open_slots: 2, .. }));
        assert!(fed.is_accepting_guardians());
        assert!(!fed.can_start_dkg());
    }

    #[test]
    fn test_consensus_config() {
        let config = ConsensusConfig::standard(4);
        assert_eq!(config.threshold, 3); // (4 * 2/3) + 1 = 3
        assert_eq!(config.total_guardians, 4);

        let config = ConsensusConfig::standard(7);
        assert_eq!(config.threshold, 5); // (7 * 2/3) + 1 = 5
    }
}