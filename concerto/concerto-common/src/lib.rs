pub mod subscription;
pub mod federation;
pub mod slot;
pub mod nostr_events;
pub mod economics;
pub mod error;
pub mod crypto;
pub mod monitoring;

// Re-export specific items to avoid conflicts
pub use subscription::*;
pub use federation::*;
pub use slot::{
    FedimintSlot, SlotState, SlotAllocation, HealthStatus, ResourceBundle,
    AllocateSlotRequest, SlotEndpoint, SlotStatus,
    // ResourceUsage is also in economics, so we'll use that one
};
pub use nostr_events::{
    FederationProposalEvent, GuardianApplicationEvent, ApplicationDecisionEvent,
    SlotAllocationEvent, DkgCoordinationEvent, DkgMessageType,
    ServiceAdvertisementEvent, ServiceType, ServiceAvailability,
    FeeStructure, PriceTier, get_federation_id_from_event,
    KIND_FEDERATION_PROPOSAL, KIND_GUARDIAN_APPLICATION, KIND_APPLICATION_DECISION,
    KIND_SLOT_ALLOCATION, KIND_DKG_COORDINATION, KIND_SERVICE_ADVERTISEMENT,
    // ServiceTerms is also in economics, so we'll use that one
};
pub use economics::*;
pub use error::*;