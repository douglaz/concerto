use concerto_common::*;
use nostr_sdk::prelude::*;
use nostr_sdk::{RelayPoolNotification, SingleLetterTag, Alphabet};
use std::time::Duration;
use tracing::{info, warn, error};
use ::url::Url;

pub struct NostrClient {
    client: Client,
    keys: Keys,
    owner_pubkey: PublicKey,
    relays: Vec<String>,
}

impl NostrClient {
    pub async fn new(
        keys: Keys,
        owner_pubkey: PublicKey,
        relays: Vec<String>,
    ) -> anyhow::Result<Self> {
        let client = Client::new(&keys);
        
        // Add relays
        for relay_url in &relays {
            match client.add_relay(relay_url).await {
                Ok(_) => info!("Connected to relay: {}", relay_url),
                Err(e) => warn!("Failed to connect to relay {}: {}", relay_url, e),
            }
        }
        
        // Connect to relays
        client.connect().await;
        
        Ok(Self {
            client,
            keys,
            owner_pubkey,
            relays,
        })
    }
    
    pub async fn subscribe_to_events(&self) -> anyhow::Result<()> {
        // Subscribe to federation-related events
        let federation_filter = Filter::new()
            .kinds(vec![
                Kind::from(KIND_FEDERATION_PROPOSAL),
                Kind::from(KIND_GUARDIAN_APPLICATION),
                Kind::from(KIND_APPLICATION_DECISION),
                Kind::from(KIND_SLOT_ALLOCATION),
                Kind::from(KIND_DKG_COORDINATION),
                Kind::from(KIND_SERVICE_ADVERTISEMENT),
            ]);
        
        // Subscribe to DMs for this guardian
        let dm_filter = Filter::new()
            .kind(Kind::EncryptedDirectMessage)
            .pubkey(self.keys.public_key());
        
        // Subscribe to events mentioning this guardian
        let mention_filter = Filter::new()
            .pubkey(self.keys.public_key());
        
        self.client.subscribe(vec![
            federation_filter,
            dm_filter,
            mention_filter,
        ], None).await?;
        
        info!("Subscribed to Nostr events");
        Ok(())
    }
    
    pub async fn next_event(&self) -> anyhow::Result<Option<Event>> {
        // Poll for new events with timeout
        // Note: In newer nostr-sdk versions, the notification API has changed
        // This is a simplified version that would need proper async handling
        Ok(None)
    }
    
    pub async fn send_dm(&self, recipient: PublicKey, message: &str) -> anyhow::Result<()> {
        let encrypted = nip04::encrypt(
            self.keys.secret_key()?,
            &recipient,
            message,
        )?;
        
        // In nostr-sdk v0.43, use text_note for now
        // TODO: Update to use proper NIP-04 encryption
        let unsigned = EventBuilder::text_note(encrypted)
            .build(self.keys.public_key());
        let event = unsigned.sign_with_keys(&self.keys)?;
        
        self.client.send_event(&event).await?;
        info!("Sent DM to {}", recipient.to_bech32()?);
        Ok(())
    }
    
    pub async fn publish_federation_proposal(
        &self,
        proposal: FederationProposalEvent,
    ) -> anyhow::Result<EventId> {
        let event = proposal.to_nostr_event(&self.keys)?;
        let output = self.client.send_event(&event).await?;
        let event_id = *output.id();
        info!("Published federation proposal: {}", event_id);
        Ok(event_id)
    }
    
    pub async fn publish_guardian_application(
        &self,
        application: GuardianApplicationEvent,
    ) -> anyhow::Result<EventId> {
        // TODO: Need to track the proposal event ID
        let proposal_event_id = EventId::from_hex("0".repeat(64))?;
        let event = application.to_nostr_event(&self.keys, proposal_event_id)?;
        let output = self.client.send_event(&event).await?;
        let event_id = *output.id();
        info!("Published guardian application: {}", event_id);
        Ok(event_id)
    }
    
    pub async fn publish_application_decision(
        &self,
        decision: ApplicationDecisionEvent,
    ) -> anyhow::Result<EventId> {
        // TODO: Need to track the application event ID and applicant pubkey
        let application_event_id = EventId::from_hex("0".repeat(64))?;
        let applicant_pubkey = PublicKey::from_hex(&decision.applicant_npub)?;
        
        let event = decision.to_nostr_event(&self.keys, application_event_id, applicant_pubkey)?;
        let output = self.client.send_event(&event).await?;
        let event_id = *output.id();
        info!("Published application decision: {}", event_id);
        Ok(event_id)
    }
    
    pub async fn publish_slot_allocation(
        &self,
        allocation: SlotAllocationEvent,
    ) -> anyhow::Result<EventId> {
        let event = allocation.to_nostr_event(&self.keys)?;
        let output = self.client.send_event(&event).await?;
        let event_id = *output.id();
        info!("Published slot allocation: {}", event_id);
        Ok(event_id)
    }
    
    pub async fn publish_dkg_coordination(
        &self,
        dkg: DkgCoordinationEvent,
    ) -> anyhow::Result<EventId> {
        let event = dkg.to_nostr_event(&self.keys)?;
        let output = self.client.send_event(&event).await?;
        let event_id = *output.id();
        info!("Published DKG coordination: {}", event_id);
        Ok(event_id)
    }
    
    pub async fn query_federation_proposals(
        &self,
        since: Option<Timestamp>,
    ) -> anyhow::Result<Vec<Event>> {
        let mut filter = Filter::new()
            .kind(Kind::from(KIND_FEDERATION_PROPOSAL));
        
        if let Some(timestamp) = since {
            filter = filter.since(timestamp);
        }
        
        let events = self.client.get_events_of(
            vec![filter],
            None,
        ).await?;
        
        Ok(events)
    }
    
    pub async fn query_federation_events(
        &self,
        federation_id: &str,
    ) -> anyhow::Result<Vec<Event>> {
        let filter = Filter::new()
            .kinds(vec![
                Kind::from(KIND_FEDERATION_PROPOSAL),
                Kind::from(KIND_GUARDIAN_APPLICATION),
                Kind::from(KIND_APPLICATION_DECISION),
                Kind::from(KIND_SLOT_ALLOCATION),
                Kind::from(KIND_DKG_COORDINATION),
            ])
            .identifier(federation_id);
        
        let events = self.client.get_events_of(
            vec![filter],
            None,
        ).await?;
        
        Ok(events)
    }
    
    pub async fn query_service_advertisements(
        &self,
        service_type: Option<&str>,
    ) -> anyhow::Result<Vec<Event>> {
        let mut filter = Filter::new()
            .kind(Kind::from(KIND_SERVICE_ADVERTISEMENT));
        
        // Note: Custom tag filtering would need to be implemented differently
        // in the newer nostr-sdk version
        
        let events = self.client.get_events_of(
            vec![filter],
            None,
        ).await?;
        
        Ok(events)
    }
}