use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConcertoError {
    #[error("Subscription error: {0}")]
    Subscription(String),
    
    #[error("Federation error: {0}")]
    Federation(String),
    
    #[error("Slot allocation error: {0}")]
    SlotAllocation(String),
    
    #[error("Nostr error: {0}")]
    Nostr(String),
    
    #[error("Economic validation failed: {0}")]
    EconomicValidation(String),
    
    #[error("Insufficient resources: {0}")]
    InsufficientResources(String),
    
    #[error("Authentication error: {0}")]
    Authentication(String),
    
    #[error("DKG error: {0}")]
    Dkg(String),
    
    #[error("Provider error: {0}")]
    Provider(String),
    
    #[error("Database error: {0}")]
    Database(String),
    
    #[error("Network error: {0}")]
    Network(String),
    
    #[error("Serialization error: {0}")]
    Serialization(String),
    
    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),
    
    #[error("Not found: {0}")]
    NotFound(String),
    
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

impl From<serde_json::Error> for ConcertoError {
    fn from(err: serde_json::Error) -> Self {
        ConcertoError::Serialization(err.to_string())
    }
}

impl From<nostr_sdk::client::Error> for ConcertoError {
    fn from(err: nostr_sdk::client::Error) -> Self {
        ConcertoError::Nostr(err.to_string())
    }
}

impl From<nostr_sdk::event::Error> for ConcertoError {
    fn from(err: nostr_sdk::event::Error) -> Self {
        ConcertoError::Nostr(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ConcertoError>;