use anyhow::Result;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Nostr bot for guardian coordination
pub struct NostrBot {
    client: Client,
    keys: Keys,
    relays: Vec<String>,
    state: Arc<RwLock<BotState>>,
}

#[derive(Debug, Clone, Default)]
struct BotState {
    // Track DMs and channels
    active_conversations: Vec<ConversationInfo>,
    // Track federation setups in progress
    active_federations: Vec<FederationSetup>,
}

#[derive(Debug, Clone)]
struct ConversationInfo {
    user_pubkey: PublicKey,
    guardian_role: GuardianRole,
    last_message_time: Timestamp,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FederationSetup {
    federation_id: String,
    lead_guardian: PublicKey,
    other_guardians: Vec<PublicKey>,
    status: SetupStatus,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum SetupStatus {
    WaitingForGuardians,
    ConfiguringServers,
    RunningDkg,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GuardianRole {
    LeadGuardian,
    OtherGuardian,
}

/// Commands that can be sent via Nostr
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NostrCommand {
    // Guardian registration
    RegisterGuardian { role: GuardianRole },

    // Federation setup
    StartFederation { name: String, num_guardians: u8 },
    JoinFederation { federation_id: String },

    // DKG coordination
    StartDkg,
    SubmitDkgShare { share: String },

    // Status queries
    GetStatus,
    ListFederations,
}

/// Events sent by the bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NostrBotEvent {
    // Registration responses
    GuardianRegistered {
        guardian_id: String,
    },

    // Federation events
    FederationCreated {
        federation_id: String,
        invite_code: String,
    },
    GuardianJoined {
        guardian_npub: String,
    },

    // DKG events
    DkgStarted,
    DkgProgress {
        current: u8,
        total: u8,
    },
    DkgComplete,

    // Status updates
    StatusUpdate {
        message: String,
    },
    Error {
        message: String,
    },
}

impl NostrBot {
    /// Create a new Nostr bot from a private key
    pub async fn new(private_key: &str, relays: Vec<String>) -> Result<Self> {
        // Parse the private key (supports both nsec and hex formats)
        let keys = Keys::parse(private_key)?;

        info!("Bot npub: {}", keys.public_key().to_bech32()?);

        // Create client
        let client = Client::new(keys.clone());

        // Add relays
        for relay_url in &relays {
            client.add_relay(relay_url).await?;
            info!("Added relay: {}", relay_url);
        }

        // Connect to relays
        client.connect().await;

        Ok(Self {
            client,
            keys,
            relays,
            state: Arc::new(RwLock::new(BotState::default())),
        })
    }

    /// Start listening for messages and commands
    pub async fn start(&self) -> Result<()> {
        info!("Starting Nostr bot...");

        // Subscribe to direct messages (NIP-04)
        let dm_filter = Filter::new()
            .kind(Kind::EncryptedDirectMessage)
            .pubkey(self.keys.public_key());

        // Subscribe to mentions
        let mention_filter = Filter::new()
            .kind(Kind::TextNote)
            .pubkey(self.keys.public_key());

        // Subscribe to our custom federation coordination events (30000-39999 range for parameterized replaceable events)
        let federation_filter = Filter::new()
            .kind(Kind::ParameterizedReplaceable(30100))
            .author(self.keys.public_key());

        let subscription_id = self
            .client
            .subscribe(vec![dm_filter, mention_filter, federation_filter], None)
            .await?;

        info!("Subscribed with ID: {:?}", subscription_id);

        // Handle events
        self.client
            .handle_notifications(|notification| async {
                if let RelayPoolNotification::Event { event, .. } = notification {
                    self.handle_event(*event).await;
                }
                Ok(false) // Continue listening
            })
            .await?;

        Ok(())
    }

    /// Handle incoming Nostr events
    async fn handle_event(&self, event: Event) {
        match event.kind {
            Kind::EncryptedDirectMessage => {
                if let Err(e) = self.handle_dm(event).await {
                    error!("Error handling DM: {}", e);
                }
            }
            Kind::TextNote => {
                if let Err(e) = self.handle_mention(event).await {
                    error!("Error handling mention: {}", e);
                }
            }
            Kind::ParameterizedReplaceable(30100) => {
                if let Err(e) = self.handle_federation_event(event).await {
                    error!("Error handling federation event: {}", e);
                }
            }
            _ => {
                debug!("Ignoring event of kind: {:?}", event.kind);
            }
        }
    }

    /// Handle direct messages
    async fn handle_dm(&self, event: Event) -> Result<()> {
        // Decrypt the message
        let decrypted = nip04::decrypt(self.keys.secret_key(), &event.pubkey, &event.content)?;

        info!(
            "Received DM from {}: {}",
            event.pubkey.to_bech32()?,
            decrypted
        );

        // Try to parse as command
        if let Ok(command) = serde_json::from_str::<NostrCommand>(&decrypted) {
            self.handle_command(event.pubkey, command).await?;
        } else {
            // Send help message if we can't parse the command
            self.send_dm(
                event.pubkey,
                "Unknown command. Send 'help' for available commands.",
            )
            .await?;
        }

        Ok(())
    }

    /// Handle mentions in public notes
    async fn handle_mention(&self, event: Event) -> Result<()> {
        info!(
            "Mentioned by {}: {}",
            event.pubkey.to_bech32()?,
            event.content
        );

        // Reply to the mention
        let tags = vec![Tag::event(event.id), Tag::public_key(event.pubkey)];
        let reply = EventBuilder::text_note(
            "Hello! I'm a Fedimint guardian bot. DM me to get started.".to_string(),
            tags,
        );
        let reply_event = self.client.sign_event_builder(reply).await?;

        self.client.send_event(reply_event).await?;
        Ok(())
    }

    /// Handle federation coordination events
    async fn handle_federation_event(&self, event: Event) -> Result<()> {
        info!("Federation event: {:?}", event);
        // TODO: Implement federation-specific event handling
        Ok(())
    }

    /// Handle parsed commands
    async fn handle_command(&self, sender: PublicKey, command: NostrCommand) -> Result<()> {
        use NostrCommand::*;

        let response = match command {
            RegisterGuardian { role } => {
                // Register the guardian with the specified role
                let guardian_id = format!("guardian_{}", sender.to_bech32()?);

                // Update state
                let mut state = self.state.write().await;
                state.active_conversations.push(ConversationInfo {
                    user_pubkey: sender,
                    guardian_role: role.clone(),
                    last_message_time: Timestamp::now(),
                });

                NostrBotEvent::GuardianRegistered { guardian_id }
            }

            StartFederation {
                name,
                num_guardians: _,
            } => {
                // Initialize a new federation setup
                let federation_id = uuid::Uuid::new_v4().to_string();

                let mut state = self.state.write().await;
                state.active_federations.push(FederationSetup {
                    federation_id: federation_id.clone(),
                    lead_guardian: sender,
                    other_guardians: Vec::new(),
                    status: SetupStatus::WaitingForGuardians,
                });

                NostrBotEvent::FederationCreated {
                    federation_id,
                    invite_code: format!("fed1_test_{}", name), // Simplified invite code
                }
            }

            GetStatus => NostrBotEvent::StatusUpdate {
                message: "Bot is operational".to_string(),
            },

            _ => NostrBotEvent::Error {
                message: "Command not yet implemented".to_string(),
            },
        };

        // Send response as DM
        let response_json = serde_json::to_string(&response)?;
        self.send_dm(sender, &response_json).await?;

        Ok(())
    }

    /// Send a direct message to a user
    async fn send_dm(&self, recipient: PublicKey, message: &str) -> Result<()> {
        let encrypted = nip04::encrypt(self.keys.secret_key(), &recipient, message)?;

        // Build encrypted direct message event (Kind 4)
        let event_builder = EventBuilder::new(
            Kind::EncryptedDirectMessage,
            encrypted,
            vec![Tag::public_key(recipient)],
        );

        let event = self.client.sign_event_builder(event_builder).await?;
        self.client.send_event(event).await?;
        info!("Sent DM to {}", recipient.to_bech32()?);
        Ok(())
    }

    /// Broadcast a public status update
    pub async fn broadcast_status(&self, message: &str) -> Result<()> {
        let event_builder = EventBuilder::text_note(message, vec![]);
        let event = self.client.sign_event_builder(event_builder).await?;
        self.client.send_event(event).await?;
        info!("Broadcasted: {}", message);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bot_creation() -> Result<()> {
        // Generate a test key
        let keys = Keys::generate();
        let private_key = keys.secret_key().to_bech32()?;

        // Use a test relay (you might want to use a mock in real tests)
        let relays = vec!["wss://relay.damus.io".to_string()];

        // This would normally connect to real relays, so we just test creation
        // In production tests, you'd want to use a mock relay
        let _bot = NostrBot::new(&private_key, relays).await?;

        Ok(())
    }
}
