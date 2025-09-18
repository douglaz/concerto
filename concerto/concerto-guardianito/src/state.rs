use concerto_common::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn, error};

pub struct StateManager {
    db: sled::Db,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredState {
    subscriptions: Vec<Subscription>,
    federations: HashMap<String, FederationParticipation>,
    slots: Vec<FedimintSlot>,
    event_cache: EventCache,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventCache {
    proposal_events: HashMap<String, String>, // federation_id -> event_id
    application_events: HashMap<String, Vec<String>>, // federation_id -> [event_ids]
    last_sync: chrono::DateTime<chrono::Utc>,
}

impl StateManager {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        let db = sled::open(db_path)?;
        info!("Opened state database at: {}", db_path);
        Ok(Self { db })
    }
    
    // Subscription management
    pub fn load_subscriptions(&self) -> anyhow::Result<Vec<Subscription>> {
        match self.db.get("subscriptions")? {
            Some(data) => {
                let subs: Vec<Subscription> = serde_json::from_slice(&data)?;
                info!("Loaded {} subscriptions from state", subs.len());
                Ok(subs)
            }
            None => {
                info!("No subscriptions found in state");
                Ok(vec![])
            }
        }
    }
    
    pub fn save_subscriptions(&self, subscriptions: &[Subscription]) -> anyhow::Result<()> {
        let data = serde_json::to_vec(subscriptions)?;
        self.db.insert("subscriptions", data)?;
        self.db.flush()?;
        info!("Saved {} subscriptions to state", subscriptions.len());
        Ok(())
    }
    
    pub fn add_subscription(&self, subscription: Subscription) -> anyhow::Result<()> {
        let mut subs = self.load_subscriptions()?;
        subs.push(subscription);
        self.save_subscriptions(&subs)?;
        Ok(())
    }
    
    pub fn remove_subscription(&self, subscription_id: uuid::Uuid) -> anyhow::Result<()> {
        let mut subs = self.load_subscriptions()?;
        subs.retain(|s| s.id != subscription_id);
        self.save_subscriptions(&subs)?;
        Ok(())
    }
    
    // Federation management
    pub fn load_federations(&self) -> anyhow::Result<HashMap<String, FederationParticipation>> {
        match self.db.get("federations")? {
            Some(data) => {
                let feds: HashMap<String, FederationParticipation> = serde_json::from_slice(&data)?;
                info!("Loaded {} federations from state", feds.len());
                Ok(feds)
            }
            None => {
                info!("No federations found in state");
                Ok(HashMap::new())
            }
        }
    }
    
    pub fn save_federations(&self, federations: &HashMap<String, FederationParticipation>) -> anyhow::Result<()> {
        let data = serde_json::to_vec(federations)?;
        self.db.insert("federations", data)?;
        self.db.flush()?;
        info!("Saved {} federations to state", federations.len());
        Ok(())
    }
    
    pub fn update_federation(&self, federation_id: String, participation: FederationParticipation) -> anyhow::Result<()> {
        let mut feds = self.load_federations()?;
        feds.insert(federation_id, participation);
        self.save_federations(&feds)?;
        Ok(())
    }
    
    // Slot management
    pub fn load_slots(&self) -> anyhow::Result<Vec<FedimintSlot>> {
        match self.db.get("slots")? {
            Some(data) => {
                let slots: Vec<FedimintSlot> = serde_json::from_slice(&data)?;
                info!("Loaded {} slots from state", slots.len());
                Ok(slots)
            }
            None => {
                info!("No slots found in state");
                Ok(vec![])
            }
        }
    }
    
    pub fn save_slots(&self, slots: &[FedimintSlot]) -> anyhow::Result<()> {
        let data = serde_json::to_vec(slots)?;
        self.db.insert("slots", data)?;
        self.db.flush()?;
        info!("Saved {} slots to state", slots.len());
        Ok(())
    }
    
    pub fn allocate_slot(&self, slot: FedimintSlot) -> anyhow::Result<()> {
        let mut slots = self.load_slots()?;
        slots.push(slot);
        self.save_slots(&slots)?;
        Ok(())
    }
    
    pub fn update_slot(&self, slot_id: uuid::Uuid, updater: impl FnOnce(&mut FedimintSlot)) -> anyhow::Result<()> {
        let mut slots = self.load_slots()?;
        if let Some(slot) = slots.iter_mut().find(|s| s.id == slot_id) {
            updater(slot);
            self.save_slots(&slots)?;
        } else {
            return Err(anyhow::anyhow!("Slot not found: {}", slot_id));
        }
        Ok(())
    }
    
    pub fn release_slot(&self, slot_id: uuid::Uuid) -> anyhow::Result<()> {
        let mut slots = self.load_slots()?;
        slots.retain(|s| s.id != slot_id);
        self.save_slots(&slots)?;
        Ok(())
    }
    
    // Event cache management
    pub fn load_event_cache(&self) -> anyhow::Result<EventCache> {
        match self.db.get("event_cache")? {
            Some(data) => {
                let cache: EventCache = serde_json::from_slice(&data)?;
                Ok(cache)
            }
            None => {
                Ok(EventCache {
                    proposal_events: HashMap::new(),
                    application_events: HashMap::new(),
                    last_sync: chrono::Utc::now(),
                })
            }
        }
    }
    
    pub fn save_event_cache(&self, cache: &EventCache) -> anyhow::Result<()> {
        let data = serde_json::to_vec(cache)?;
        self.db.insert("event_cache", data)?;
        self.db.flush()?;
        Ok(())
    }
    
    pub fn cache_proposal_event(&self, federation_id: String, event_id: String) -> anyhow::Result<()> {
        let mut cache = self.load_event_cache()?;
        cache.proposal_events.insert(federation_id, event_id);
        cache.last_sync = chrono::Utc::now();
        self.save_event_cache(&cache)?;
        Ok(())
    }
    
    pub fn cache_application_event(&self, federation_id: String, event_id: String) -> anyhow::Result<()> {
        let mut cache = self.load_event_cache()?;
        cache.application_events
            .entry(federation_id)
            .or_insert_with(Vec::new)
            .push(event_id);
        cache.last_sync = chrono::Utc::now();
        self.save_event_cache(&cache)?;
        Ok(())
    }
    
    pub fn get_proposal_event_id(&self, federation_id: &str) -> anyhow::Result<Option<String>> {
        let cache = self.load_event_cache()?;
        Ok(cache.proposal_events.get(federation_id).cloned())
    }
    
    pub fn get_application_event_ids(&self, federation_id: &str) -> anyhow::Result<Vec<String>> {
        let cache = self.load_event_cache()?;
        Ok(cache.application_events.get(federation_id).cloned().unwrap_or_default())
    }
    
    // Full state management
    pub fn export_state(&self) -> anyhow::Result<StoredState> {
        Ok(StoredState {
            subscriptions: self.load_subscriptions()?,
            federations: self.load_federations()?,
            slots: self.load_slots()?,
            event_cache: self.load_event_cache()?,
        })
    }
    
    pub fn import_state(&self, state: StoredState) -> anyhow::Result<()> {
        self.save_subscriptions(&state.subscriptions)?;
        self.save_federations(&state.federations)?;
        self.save_slots(&state.slots)?;
        self.save_event_cache(&state.event_cache)?;
        info!("Imported full state");
        Ok(())
    }
    
    pub fn clear_state(&self) -> anyhow::Result<()> {
        self.db.clear()?;
        self.db.flush()?;
        warn!("Cleared all state");
        Ok(())
    }
}