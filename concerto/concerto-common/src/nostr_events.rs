use nostr_sdk::prelude::*;
use nostr_sdk::event::builder::Error as BuilderError;
use serde::{Deserialize, Serialize};
use ::url::Url;
use std::borrow::Cow;

// Custom event kinds for Concerto (30500-30504)
pub const KIND_FEDERATION_PROPOSAL: u16 = 30500;
pub const KIND_GUARDIAN_APPLICATION: u16 = 30501;
pub const KIND_APPLICATION_DECISION: u16 = 30502;
pub const KIND_SLOT_ALLOCATION: u16 = 30503;
pub const KIND_DKG_COORDINATION: u16 = 30504;
pub const KIND_SERVICE_ADVERTISEMENT: u16 = 30600;

/// Federation Proposal Event (Kind 30500)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationProposalEvent {
    pub federation_id: String,
    pub name: String,
    pub description: Option<String>,
    pub initiator_slots: u32,
    pub total_slots: u32,
    pub requirements: crate::FederationRequirements,
    pub consensus_config: crate::ConsensusConfig,
}

impl FederationProposalEvent {
    pub fn to_nostr_event(&self, keys: &Keys) -> Result<Event, BuilderError> {
        let content = serde_json::to_string(self).unwrap();
        
        let tags = vec![
            Tag::identifier(self.federation_id.clone()),
            Tag::custom(TagKind::Custom(Cow::Borrowed("name")), vec![self.name.clone()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("slots")), vec![self.total_slots.to_string()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("open")), vec![(self.total_slots - self.initiator_slots).to_string()]),
        ];

        let unsigned = EventBuilder::new(Kind::from(KIND_FEDERATION_PROPOSAL), content)
            .tags(tags)
            .build(keys.public_key());
        Ok(unsigned.sign_with_keys(keys)?)
    }

    pub fn from_nostr_event(event: &Event) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(&event.content)?)
    }
}

/// Guardian Application Event (Kind 30501)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianApplicationEvent {
    pub federation_id: String,
    pub applicant_npub: String,
    pub slots_to_contribute: u32,
    pub preferred_providers: Vec<Url>,
    pub message: Option<String>,
    pub subscription_proof: Option<crate::SubscriptionProof>,
}

impl GuardianApplicationEvent {
    pub fn to_nostr_event(&self, keys: &Keys, proposal_event_id: EventId) -> Result<Event, BuilderError> {
        let content = serde_json::to_string(self).unwrap();
        
        let tags = vec![
            Tag::event(proposal_event_id),
            Tag::custom(TagKind::Custom(Cow::Borrowed("federation")), vec![self.federation_id.clone()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("slots")), vec![self.slots_to_contribute.to_string()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("providers")), vec![
                self.preferred_providers.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",")
            ]),
        ];

        let unsigned = EventBuilder::new(Kind::from(KIND_GUARDIAN_APPLICATION), content)
            .tags(tags)
            .build(keys.public_key());
        Ok(unsigned.sign_with_keys(keys)?)
    }

    pub fn from_nostr_event(event: &Event) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(&event.content)?)
    }
}

/// Application Decision Event (Kind 30502)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationDecisionEvent {
    pub application_event_id: String,
    pub federation_id: String,
    pub applicant_npub: String,
    pub decision: crate::Decision,
    pub message: Option<String>,
}

impl ApplicationDecisionEvent {
    pub fn to_nostr_event(&self, keys: &Keys, application_event_id: EventId, applicant_pubkey: PublicKey) -> Result<Event, BuilderError> {
        let content = serde_json::to_string(self).unwrap();
        
        let decision_str = match &self.decision {
            crate::Decision::Approved => "approved",
            crate::Decision::Rejected { .. } => "rejected",
        };
        
        let tags = vec![
            Tag::event(application_event_id),
            Tag::custom(TagKind::Custom(Cow::Borrowed("federation")), vec![self.federation_id.clone()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("decision")), vec![decision_str.to_string()]),
            Tag::public_key(applicant_pubkey), // Notify applicant
        ];

        let unsigned = EventBuilder::new(Kind::from(KIND_APPLICATION_DECISION), content)
            .tags(tags)
            .build(keys.public_key());
        Ok(unsigned.sign_with_keys(keys)?)
    }

    pub fn from_nostr_event(event: &Event) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(&event.content)?)
    }
}

/// Slot Allocation Event (Kind 30503)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotAllocationEvent {
    pub federation_id: String,
    pub slot_id: uuid::Uuid,
    pub guardian_npub: String,
    pub provider_url: Url,
    pub endpoint: Option<Url>,
    pub status: crate::SlotState,
}

impl SlotAllocationEvent {
    pub fn to_nostr_event(&self, keys: &Keys) -> Result<Event, BuilderError> {
        let content = serde_json::to_string(self).unwrap();
        
        let mut tags = vec![
            Tag::custom(TagKind::Custom(Cow::Borrowed("federation")), vec![self.federation_id.clone()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("slot")), vec![self.slot_id.to_string()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("provider")), vec![self.provider_url.to_string()]),
        ];
        
        if let Some(endpoint) = &self.endpoint {
            tags.push(Tag::custom(TagKind::Custom(Cow::Borrowed("endpoint")), vec![endpoint.to_string()]));
        }

        let unsigned = EventBuilder::new(Kind::from(KIND_SLOT_ALLOCATION), content)
            .tags(tags)
            .build(keys.public_key());
        Ok(unsigned.sign_with_keys(keys)?)
    }

    pub fn from_nostr_event(event: &Event) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(&event.content)?)
    }
}

/// DKG Coordination Event (Kind 30504)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DkgCoordinationEvent {
    pub federation_id: String,
    pub round: u32,
    pub message_type: DkgMessageType,
    pub payload: String, // Encrypted DKG data
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DkgMessageType {
    Initiate { participants: Vec<String> },
    SetupCode, // Guardian sharing their Fedimint setup code
    Complete { success: bool },
}

impl DkgCoordinationEvent {
    pub fn to_nostr_event(&self, keys: &Keys) -> Result<Event, BuilderError> {
        let content = serde_json::to_string(self).unwrap();
        
        let msg_type = match &self.message_type {
            DkgMessageType::Initiate { .. } => "initiate",
            DkgMessageType::SetupCode => "setup_code",
            DkgMessageType::Complete { .. } => "complete",
        };
        
        let tags = vec![
            Tag::custom(TagKind::Custom(Cow::Borrowed("federation")), vec![self.federation_id.clone()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("round")), vec![self.round.to_string()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("type")), vec![msg_type.to_string()]),
        ];

        let unsigned = EventBuilder::new(Kind::from(KIND_DKG_COORDINATION), content)
            .tags(tags)
            .build(keys.public_key());
        Ok(unsigned.sign_with_keys(keys)?)
    }

    pub fn from_nostr_event(event: &Event) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(&event.content)?)
    }
}

/// Service Advertisement Event (Kind 30600)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAdvertisementEvent {
    pub provider_npub: String,
    pub service_type: ServiceType,
    pub terms: ServiceTerms,
    pub supported_federations: Option<Vec<String>>,
    pub requirements: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceType {
    LightningGateway {
        capacity_sats: u64,
        fee_rate: f64,
    },
    StabilityPool {
        available_liquidity: u64,
        collateral_ratio: f64,
    },
    OnchainServices {
        services: Vec<String>,
    },
    FeLaaSProvider {
        regions: Vec<String>,
        pricing: crate::PricingModel,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceTerms {
    pub min_federation_size: u32,
    pub fee_structure: FeeStructure,
    pub availability: ServiceAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FeeStructure {
    Flat { sats_per_month: u64 },
    Percentage { basis_points: u32 },
    Tiered { tiers: Vec<PriceTier> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceTier {
    pub up_to_amount_sats: u64,
    pub fee_basis_points: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceAvailability {
    Always,
    Hours { start: u8, end: u8 },
    OnDemand,
}

impl ServiceAdvertisementEvent {
    pub fn to_nostr_event(&self, keys: &Keys) -> Result<Event, BuilderError> {
        let content = serde_json::to_string(self).unwrap();
        
        let service_type_str = match &self.service_type {
            ServiceType::LightningGateway { .. } => "lightning_gateway",
            ServiceType::StabilityPool { .. } => "stability_pool",
            ServiceType::OnchainServices { .. } => "onchain_services",
            ServiceType::FeLaaSProvider { .. } => "felaas_provider",
        };
        
        let tags = vec![
            Tag::custom(TagKind::Custom(Cow::Borrowed("service")), vec![service_type_str.to_string()]),
            Tag::custom(TagKind::Custom(Cow::Borrowed("provider")), vec![self.provider_npub.clone()]),
        ];

        let unsigned = EventBuilder::new(Kind::from(KIND_SERVICE_ADVERTISEMENT), content)
            .tags(tags)
            .build(keys.public_key());
        Ok(unsigned.sign_with_keys(keys)?)
    }

    pub fn from_nostr_event(event: &Event) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(&event.content)?)
    }
}

/// Helper to extract federation ID from event tags
pub fn get_federation_id_from_event(event: &Event) -> Option<String> {
    // In nostr-sdk 0.34, custom tags are accessed differently
    // We need to iterate through tags and check their kind
    for tag in event.tags.iter() {
        if tag.kind() == TagKind::Custom(Cow::Borrowed("federation")) {
            if let Some(content) = tag.content() {
                return Some(content.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_federation_proposal_event() {
        let keys = Keys::generate();
        
        let proposal = FederationProposalEvent {
            federation_id: "fed123".to_string(),
            name: "Test Federation".to_string(),
            description: Some("A test federation".to_string()),
            initiator_slots: 2,
            total_slots: 4,
            requirements: crate::FederationRequirements {
                min_slots: 1,
                min_subscription_tier: None,
                required_features: vec![],
                geographic_diversity: None,
                custom_requirements: std::collections::HashMap::new(),
            },
            consensus_config: crate::ConsensusConfig::standard(4),
        };

        let event = proposal.to_nostr_event(&keys).unwrap();
        assert_eq!(event.kind, Kind::from(KIND_FEDERATION_PROPOSAL));
        
        let decoded = FederationProposalEvent::from_nostr_event(&event).unwrap();
        assert_eq!(decoded.federation_id, proposal.federation_id);
        assert_eq!(decoded.name, proposal.name);
    }
}