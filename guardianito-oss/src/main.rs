use anyhow::Result;
use clap::{Parser, Subcommand};
use guardianito_oss::nostr::NostrBot;
use tracing::{error, info};
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[command(name = "guardianito-oss")]
#[command(about = "Open Source Fedimint Guardian Bot - Nostr-based coordination")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Commands {
    /// Run the guardian daemon
    Daemon {
        /// Nostr relay URLs
        #[arg(long, value_delimiter = ',')]
        relays: Vec<String>,

        /// Nostr private key (nsec or hex)
        #[arg(long, env = "NOSTR_PRIVATE_KEY")]
        private_key: String,

        /// Storage path for bot state
        #[arg(long, default_value = "/tmp/guardianito-oss")]
        store_path: String,

        /// PostgreSQL host
        #[arg(long, env = "PGHOST", default_value = "localhost")]
        pghost: String,

        /// PostgreSQL port
        #[arg(long, env = "PGPORT", default_value = "5432")]
        pgport: u16,

        /// PostgreSQL user
        #[arg(long, env = "PGUSER", default_value = "guardianito")]
        pguser: String,

        /// PostgreSQL password
        #[arg(long, env = "PGPASSWORD")]
        pgpassword: String,

        /// PostgreSQL database
        #[arg(long, env = "PGDATABASE", default_value = "guardianito")]
        pgdatabase: String,

        /// PostgreSQL schema
        #[arg(long, env = "PGSCHEMA", default_value = "public")]
        pgschema: String,

        /// FeLaaS API URL
        #[arg(long, env = "FELAAS_URL", default_value = "http://localhost:3001")]
        felaas_url: String,

        /// API bind address for public endpoints
        #[arg(long, default_value = "[::]:3000")]
        api_bind: String,

        /// API bind address for internal endpoints
        #[arg(long, default_value = "[::]:3001")]
        api_internal_bind: String,

        /// Admin token for internal API
        #[arg(long, env = "ADMIN_TOKEN")]
        admin_token: String,
    },

    /// Create or retrieve a bot for a user
    CreateBot {
        /// Nostr npub of the user
        #[arg(long)]
        user_npub: String,

        /// Role of the guardian (LG or OG)
        #[arg(long, value_enum)]
        role: GuardianRole,

        /// API URL
        #[arg(long, default_value = "http://localhost:3000")]
        api_url: String,
    },

    /// List active bots
    ListBots {
        /// API URL
        #[arg(long, default_value = "http://localhost:3001")]
        api_url: String,

        /// Admin token for internal API
        #[arg(long, env = "ADMIN_TOKEN")]
        admin_token: String,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum GuardianRole {
    /// Lead Guardian
    Lg,
    /// Other Guardian
    Og,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon {
            relays,
            private_key,
            ..
        } => {
            info!("Starting Guardianito-OSS daemon");
            info!("Connecting to Nostr relays: {:?}", relays);

            // Create and start the Nostr bot
            let bot = NostrBot::new(&private_key, relays).await?;

            // Start listening for messages
            if let Err(e) = bot.start().await {
                error!("Bot error: {}", e);
                return Err(e);
            }

            Ok(())
        }
        Commands::CreateBot {
            user_npub, role, ..
        } => {
            info!("Creating bot for user {} with role {:?}", user_npub, role);
            // TODO: Implement bot creation via API
            Ok(())
        }
        Commands::ListBots { .. } => {
            info!("Listing active bots");
            // TODO: Implement bot listing via API
            Ok(())
        }
    }
}
