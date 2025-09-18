mod guardianito;
mod nostr_client;
mod state;
mod commands;
mod dkg;
mod config_exchange;
mod activation;
mod discovery;

use clap::{Parser, Subcommand};
use nostr_sdk::prelude::*;
use tracing::{info, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "guardianito")]
#[command(about = "Concerto Guardian Tool - Manage federations via Nostr", long_about = None)]
struct Cli {
    /// Owner's Nostr public key (npub or hex) - bot will only respond to this user
    #[arg(long, env = "OWNER_NPUB")]
    owner_npub: String,
    
    /// Nostr private key (nsec or hex) for the guardian
    #[arg(long, env = "GUARDIAN_NSEC")]
    guardian_nsec: Option<String>,
    
    /// Nostr relay URLs (comma-separated)
    #[arg(long, env = "NOSTR_RELAYS", default_value = "wss://relay.damus.io,wss://relay.nostr.band")]
    relays: String,
    
    /// State database path
    #[arg(long, env = "STATE_DB_PATH", default_value = "./guardianito.db")]
    state_db: String,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run guardian in daemon mode
    Daemon,
    
    /// Subscription management
    Subscription {
        #[command(subcommand)]
        action: SubscriptionCommands,
    },
    
    /// Federation management
    Federation {
        #[command(subcommand)]
        action: FederationCommands,
    },
    
    /// Slot management
    Slot {
        #[command(subcommand)]
        action: SlotCommands,
    },
}

#[derive(Subcommand)]
enum SubscriptionCommands {
    /// List active subscriptions
    List,
    /// Show available slots
    Available,
    /// Purchase a new subscription
    Purchase {
        plan: String,
    },
}

#[derive(Subcommand)]
enum FederationCommands {
    /// Propose a new federation
    Propose {
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, default_value = "1")]
        my_slots: u32,
        #[arg(long, default_value = "4")]
        total_slots: u32,
    },
    
    /// Apply to join a federation
    Apply {
        federation_id: String,
        #[arg(long, default_value = "1")]
        slots: u32,
        #[arg(long)]
        message: Option<String>,
    },
    
    /// Approve a guardian application
    Approve {
        federation_id: String,
        guardian_npub: String,
    },
    
    /// Reject a guardian application
    Reject {
        federation_id: String,
        guardian_npub: String,
        #[arg(long)]
        reason: String,
    },
    
    /// List federations
    List {
        #[arg(long)]
        all: bool,
    },
    
    /// Show federation status
    Status {
        federation_id: String,
    },
}

#[derive(Subcommand)]
enum SlotCommands {
    /// Allocate slots to a federation
    Allocate {
        federation_id: String,
        #[arg(long)]
        count: u32,
        #[arg(long)]
        provider: String,
    },
    
    /// Release slots from a federation
    Release {
        slot_ids: Vec<String>,
    },
    
    /// List allocated slots
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "guardianito=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    
    // Parse owner public key
    let owner_pubkey = if cli.owner_npub.starts_with("npub") {
        PublicKey::from_bech32(&cli.owner_npub)?
    } else {
        PublicKey::from_hex(&cli.owner_npub)?
    };
    
    info!("Guardianito starting with owner: {}", owner_pubkey.to_bech32()?);
    
    // Parse or generate guardian keys
    let keys = if let Some(nsec) = cli.guardian_nsec {
        if nsec.starts_with("nsec") {
            Keys::from_str(&nsec)?
        } else {
            let secret_key = SecretKey::from_hex(&nsec)?;
            Keys::new(secret_key)
        }
    } else {
        info!("No private key provided, generating new keys");
        Keys::generate()
    };
    
    info!("Guardian public key: {}", keys.public_key().to_bech32()?);
    
    // Parse relay URLs
    let relays: Vec<String> = cli.relays.split(',').map(|s| s.trim().to_string()).collect();
    
    // Initialize state database
    let state = state::StateManager::new(&cli.state_db)?;
    
    // Create guardianito instance
    let mut guardian = guardianito::Guardianito::new(
        keys,
        owner_pubkey,
        relays.clone(),
        state,
    ).await?;
    
    // Execute command
    match cli.command {
        Commands::Daemon => {
            info!("Starting Guardianito daemon mode");
            guardian.run_daemon().await?;
        }
        Commands::Subscription { action } => {
            handle_subscription_command(&mut guardian, action).await?;
        }
        Commands::Federation { action } => {
            handle_federation_command(&mut guardian, action).await?;
        }
        Commands::Slot { action } => {
            handle_slot_command(&mut guardian, action).await?;
        }
    }
    
    Ok(())
}

async fn handle_subscription_command(
    guardian: &mut guardianito::Guardianito,
    action: SubscriptionCommands,
) -> anyhow::Result<()> {
    match action {
        SubscriptionCommands::List => {
            let subs = guardian.list_subscriptions().await?;
            println!("Active subscriptions:");
            for sub in subs {
                println!("  - {} (expires: {})", sub.id, sub.valid_until);
            }
        }
        SubscriptionCommands::Available => {
            let slots = guardian.get_available_slots().await?;
            println!("Available slots: {}", slots);
        }
        SubscriptionCommands::Purchase { plan } => {
            println!("Purchasing subscription plan: {}", plan);
            // TODO: Implement purchase flow
        }
    }
    Ok(())
}

async fn handle_federation_command(
    guardian: &mut guardianito::Guardianito,
    action: FederationCommands,
) -> anyhow::Result<()> {
    match action {
        FederationCommands::Propose { name, description, my_slots, total_slots } => {
            let fed_id = guardian.propose_federation(
                name,
                description,
                my_slots,
                total_slots,
            ).await?;
            println!("Federation proposed with ID: {}", fed_id);
        }
        FederationCommands::Apply { federation_id, slots, message } => {
            guardian.apply_to_federation(
                federation_id.clone(),
                slots,
                message,
            ).await?;
            println!("Applied to federation: {}", federation_id);
        }
        FederationCommands::Approve { federation_id, guardian_npub } => {
            guardian.approve_guardian(
                federation_id.clone(),
                guardian_npub.clone(),
            ).await?;
            println!("Approved guardian {} for federation {}", guardian_npub, federation_id);
        }
        FederationCommands::Reject { federation_id, guardian_npub, reason } => {
            guardian.reject_guardian(
                federation_id.clone(),
                guardian_npub.clone(),
                reason.clone(),
            ).await?;
            println!("Rejected guardian {} for federation {}", guardian_npub, federation_id);
        }
        FederationCommands::List { all } => {
            let feds = if all {
                guardian.list_all_federations().await?
            } else {
                guardian.list_my_federations().await?
            };
            
            println!("Federations:");
            for fed in feds {
                println!("  - {} ({}): {:?}", fed.id, fed.name, fed.status);
            }
        }
        FederationCommands::Status { federation_id } => {
            let status = guardian.get_federation_status(federation_id.clone()).await?;
            println!("Federation {} status: {:?}", federation_id, status);
        }
    }
    Ok(())
}

async fn handle_slot_command(
    guardian: &mut guardianito::Guardianito,
    action: SlotCommands,
) -> anyhow::Result<()> {
    match action {
        SlotCommands::Allocate { federation_id, count, provider } => {
            let slots = guardian.allocate_slots(
                federation_id.clone(),
                count,
                url::Url::parse(&provider)?,
            ).await?;
            println!("Allocated {} slots to federation {}", slots.len(), federation_id);
        }
        SlotCommands::Release { slot_ids } => {
            let slot_uuids: Vec<uuid::Uuid> = slot_ids
                .iter()
                .map(|s| uuid::Uuid::parse_str(s))
                .collect::<Result<Vec<_>, _>>()?;
            
            guardian.release_slots(slot_uuids).await?;
            println!("Released {} slots", slot_ids.len());
        }
        SlotCommands::List => {
            let slots = guardian.list_allocated_slots().await?;
            println!("Allocated slots:");
            for slot in slots {
                println!("  - {}: {:?}", slot.id, slot.state);
            }
        }
    }
    Ok(())
}