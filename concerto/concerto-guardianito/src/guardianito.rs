use concerto_common::*;
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use tracing::{info, warn, error};
use ::url::Url;
use uuid::Uuid;

pub struct Guardianito {
    // Identity
    keys: Keys,
    owner_npub: PublicKey,
    
    // Nostr connection
    nostr_client: crate::nostr_client::NostrClient,
    
    // State
    state: crate::state::StateManager,
    
    // Runtime data
    subscriptions: Vec<Subscription>,
    federations: HashMap<String, FederationParticipation>,
}

impl Guardianito {
    pub async fn new(
        keys: Keys,
        owner_npub: PublicKey,
        relays: Vec<String>,
        state: crate::state::StateManager,
    ) -> anyhow::Result<Self> {
        let nostr_client = crate::nostr_client::NostrClient::new(
            keys.clone(),
            owner_npub,
            relays,
        ).await?;
        
        // Load state from database
        let subscriptions = state.load_subscriptions()?;
        let federations = state.load_federations()?;
        
        Ok(Self {
            keys,
            owner_npub,
            nostr_client,
            state,
            subscriptions,
            federations,
        })
    }
    
    pub async fn run_daemon(&mut self) -> anyhow::Result<()> {
        info!("Starting Guardianito daemon");
        
        // Subscribe to relevant Nostr events
        self.nostr_client.subscribe_to_events().await?;
        
        // Main event loop
        loop {
            // Process incoming events
            if let Some(event) = self.nostr_client.next_event().await? {
                self.handle_event(event).await?;
            }
            
            // Check for state updates
            self.check_state_updates().await?;
            
            // Small delay to prevent busy loop
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
    
    async fn handle_event(&mut self, event: Event) -> anyhow::Result<()> {
        match event.kind.as_u16() {
            KIND_FEDERATION_PROPOSAL => {
                self.handle_federation_proposal(event).await?;
            }
            KIND_GUARDIAN_APPLICATION => {
                self.handle_guardian_application(event).await?;
            }
            KIND_APPLICATION_DECISION => {
                self.handle_application_decision(event).await?;
            }
            KIND_SLOT_ALLOCATION => {
                self.handle_slot_allocation(event).await?;
            }
            KIND_DKG_COORDINATION => {
                self.handle_dkg_coordination(event).await?;
            }
            _ => {
                // Handle DMs from owner
                if event.kind == Kind::EncryptedDirectMessage {
                    self.handle_owner_dm(event).await?;
                }
            }
        }
        Ok(())
    }
    
    async fn handle_owner_dm(&mut self, event: Event) -> anyhow::Result<()> {
        // Only process DMs from owner
        if event.pubkey != self.owner_npub {
            warn!("Ignoring DM from non-owner: {}", event.pubkey);
            return Ok(());
        }
        
        // Decrypt and process command
        let decrypted = nip04::decrypt(
            self.keys.secret_key()?,
            &event.pubkey,
            &event.content,
        )?;
        
        info!("Received command from owner: {}", decrypted);
        
        // Parse and execute command
        let response = self.execute_command(&decrypted).await?;
        
        // Send response back to owner
        self.nostr_client.send_dm(self.owner_npub, &response).await?;
        
        Ok(())
    }
    
    async fn execute_command(&mut self, command: &str) -> anyhow::Result<String> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        
        match parts.get(0) {
            Some(&"help") => {
                Ok("Available commands: subscriptions, federations, slots, propose, apply, approve, reject".to_string())
            }
            Some(&"subscriptions") => {
                let subs = self.list_subscriptions().await?;
                Ok(format!("Active subscriptions: {}", subs.len()))
            }
            Some(&"federations") => {
                let feds = self.list_my_federations().await?;
                Ok(format!("My federations: {}", feds.len()))
            }
            Some(&"slots") => {
                let slots = self.get_available_slots().await?;
                Ok(format!("Available slots: {}", slots))
            }
            _ => {
                Ok("Unknown command. Type 'help' for available commands.".to_string())
            }
        }
    }
    
    // Subscription management
    pub async fn list_subscriptions(&self) -> anyhow::Result<Vec<Subscription>> {
        Ok(self.subscriptions.clone())
    }
    
    pub async fn get_available_slots(&self) -> anyhow::Result<u32> {
        let total_slots: u32 = self.subscriptions
            .iter()
            .filter(|s| s.is_active())
            .map(|s| match &s.plan {
                SubscriptionPlan::SlotBased(info) => info.total_slots,
            })
            .sum();
        
        let used_slots: u32 = self.federations
            .values()
            .map(|f| f.my_slots.len() as u32)
            .sum();
        
        Ok(total_slots.saturating_sub(used_slots))
    }
    
    // Federation management
    pub async fn propose_federation(
        &mut self,
        name: String,
        description: Option<String>,
        my_slots: u32,
        total_slots: u32,
    ) -> anyhow::Result<String> {
        // Check available slots
        let available = self.get_available_slots().await?;
        if available < my_slots {
            return Err(anyhow::anyhow!("Insufficient slots: {} available, {} required", available, my_slots));
        }
        
        // Create federation proposal
        let federation_id = format!("fed_{}", Uuid::new_v4());
        let proposal = FederationProposalEvent {
            federation_id: federation_id.clone(),
            name,
            description,
            initiator_slots: my_slots,
            total_slots,
            requirements: FederationRequirements {
                min_slots: 1,
                min_subscription_tier: None,
                required_features: vec![],
                geographic_diversity: None,
                custom_requirements: HashMap::new(),
            },
            consensus_config: ConsensusConfig::standard(total_slots),
        };
        
        // Publish to Nostr
        self.nostr_client.publish_federation_proposal(proposal).await?;
        
        // Update local state
        let participation = FederationParticipation {
            federation_id: federation_id.clone(),
            my_role: ParticipantRole::Initiator,
            my_slots: vec![],
            other_guardians: vec![],
        };
        
        self.federations.insert(federation_id.clone(), participation);
        self.state.save_federations(&self.federations)?;
        
        Ok(federation_id)
    }
    
    pub async fn apply_to_federation(
        &mut self,
        federation_id: String,
        slots: u32,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        // Check available slots
        let available = self.get_available_slots().await?;
        if available < slots {
            return Err(anyhow::anyhow!("Insufficient slots: {} available, {} required", available, slots));
        }
        
        // Create application
        let application = GuardianApplicationEvent {
            federation_id: federation_id.clone(),
            applicant_npub: self.keys.public_key().to_string(),
            slots_to_contribute: slots,
            preferred_providers: vec![],
            message,
            subscription_proof: None, // TODO: Add subscription proof
        };
        
        // Publish to Nostr
        self.nostr_client.publish_guardian_application(application).await?;
        
        // Update local state
        let participation = FederationParticipation {
            federation_id: federation_id.clone(),
            my_role: ParticipantRole::Candidate,
            my_slots: vec![],
            other_guardians: vec![],
        };
        
        self.federations.insert(federation_id, participation);
        self.state.save_federations(&self.federations)?;
        
        Ok(())
    }
    
    pub async fn approve_guardian(
        &mut self,
        federation_id: String,
        guardian_npub: String,
    ) -> anyhow::Result<()> {
        // Check if we're the initiator
        let participation = self.federations.get(&federation_id)
            .ok_or_else(|| anyhow::anyhow!("Not participating in federation"))?;
        
        if participation.my_role != ParticipantRole::Initiator {
            return Err(anyhow::anyhow!("Only initiator can approve guardians"));
        }
        
        // Create approval decision
        let decision = ApplicationDecisionEvent {
            application_event_id: Uuid::new_v4().to_string(), // TODO: Track actual event IDs
            federation_id,
            applicant_npub: guardian_npub,
            decision: Decision::Approved,
            message: None,
        };
        
        // Publish to Nostr
        self.nostr_client.publish_application_decision(decision).await?;
        
        Ok(())
    }
    
    pub async fn reject_guardian(
        &mut self,
        federation_id: String,
        guardian_npub: String,
        reason: String,
    ) -> anyhow::Result<()> {
        // Check if we're the initiator
        let participation = self.federations.get(&federation_id)
            .ok_or_else(|| anyhow::anyhow!("Not participating in federation"))?;
        
        if participation.my_role != ParticipantRole::Initiator {
            return Err(anyhow::anyhow!("Only initiator can reject guardians"));
        }
        
        // Create rejection decision
        let decision = ApplicationDecisionEvent {
            application_event_id: Uuid::new_v4().to_string(), // TODO: Track actual event IDs
            federation_id,
            applicant_npub: guardian_npub,
            decision: Decision::Rejected { reason },
            message: None,
        };
        
        // Publish to Nostr
        self.nostr_client.publish_application_decision(decision).await?;
        
        Ok(())
    }
    
    pub async fn list_my_federations(&self) -> anyhow::Result<Vec<Federation>> {
        // TODO: Reconstruct from Nostr events
        Ok(vec![])
    }
    
    pub async fn list_all_federations(&self) -> anyhow::Result<Vec<Federation>> {
        // TODO: Query Nostr for all federation proposals
        Ok(vec![])
    }
    
    pub async fn get_federation_status(&self, federation_id: String) -> anyhow::Result<FederationStatus> {
        // TODO: Query Nostr for federation status
        Ok(FederationStatus::Proposed { total_slots: 4, open_slots: 2 })
    }
    
    // Slot management
    pub async fn allocate_slots(
        &mut self,
        federation_id: String,
        count: u32,
        provider: Url,
    ) -> anyhow::Result<Vec<FedimintSlot>> {
        // TODO: Implement slot allocation
        Ok(vec![])
    }
    
    pub async fn release_slots(&mut self, slot_ids: Vec<Uuid>) -> anyhow::Result<()> {
        // TODO: Implement slot release
        Ok(())
    }
    
    pub async fn list_allocated_slots(&self) -> anyhow::Result<Vec<FedimintSlot>> {
        // TODO: Get slots from state
        Ok(vec![])
    }
    
    // Event handlers
    async fn handle_federation_proposal(&mut self, event: Event) -> anyhow::Result<()> {
        let proposal = FederationProposalEvent::from_nostr_event(&event)?;
        info!("New federation proposal: {} - {}", proposal.federation_id, proposal.name);
        // TODO: Store and process proposal
        Ok(())
    }
    
    async fn handle_guardian_application(&mut self, event: Event) -> anyhow::Result<()> {
        let application = GuardianApplicationEvent::from_nostr_event(&event)?;
        info!("Guardian application for federation: {}", application.federation_id);
        // TODO: Process application if we're the initiator
        Ok(())
    }
    
    async fn handle_application_decision(&mut self, event: Event) -> anyhow::Result<()> {
        let decision = ApplicationDecisionEvent::from_nostr_event(&event)?;
        info!("Application decision for federation: {}", decision.federation_id);
        // TODO: Update local state based on decision
        Ok(())
    }
    
    async fn handle_slot_allocation(&mut self, event: Event) -> anyhow::Result<()> {
        let allocation = SlotAllocationEvent::from_nostr_event(&event)?;
        info!("Slot allocation for federation: {}", allocation.federation_id);
        // TODO: Process slot allocation
        Ok(())
    }
    
    async fn handle_dkg_coordination(&mut self, event: Event) -> anyhow::Result<()> {
        let dkg = DkgCoordinationEvent::from_nostr_event(&event)?;
        info!("DKG coordination for federation: {}", dkg.federation_id);
        // TODO: Process DKG messages
        Ok(())
    }
    
    async fn check_state_updates(&mut self) -> anyhow::Result<()> {
        // TODO: Check for subscription expiry, federation status changes, etc.
        Ok(())
    }
}