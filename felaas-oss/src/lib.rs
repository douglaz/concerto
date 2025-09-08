use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::Args;
use fedimint_core::invite_code::InviteCode;

// Core modules
pub mod amount;
pub mod api;
pub mod common;
pub mod federation_launcher_daemon;
pub mod fedimint_status_client;
pub mod guardian_launcher_tng;
pub mod launch;
pub mod og_registry;
pub mod subscription_daemon;
pub mod wallet;

// Database types
pub type PgPool = deadpool_postgres::Pool;
pub type PgClient = deadpool_postgres::Object;

// Re-exports for convenience
pub use amount::Amount;
pub use common::{ChatUserId, SubscriptionStatus, create_pg_pool};

// Initialize logging
pub fn initialize_logging() {
    use tracing_subscriber::prelude::*;

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

// Database configuration
#[derive(Debug, Clone, Args)]
pub struct PgParams {
    #[clap(long, help = "Postgres host", env = "PGHOST")]
    pub pghost: String,

    #[clap(long, help = "Postgres port", env = "PGPORT")]
    pub pgport: String,

    #[clap(long, help = "Postgres user", env = "PGUSER")]
    pub pguser: String,

    #[clap(long, help = "Postgres password", env = "PGPASSWORD")]
    pub pgpassword: Option<String>,

    #[clap(long, help = "Postgres database", env = "PGDATABASE")]
    pub pgdatabase: String,

    #[clap(long, help = "Postgres schema", env = "PGSCHEMA")]
    pub pgschema: String,
}

// API server arguments
#[derive(Debug, Args, Clone)]
pub struct ApiArgs {
    #[clap(
        long,
        default_value = "[::]:3000",
        help = "Address to bind/listen to",
        env = "BIND_ADDRESS"
    )]
    pub bind: SocketAddr,

    #[clap(
        long,
        help = "Invite code for the wallet federation",
        env = "WALLET_FEDERATION_INVITE_CODE"
    )]
    pub wallet_federation_invite_code: InviteCode,

    #[clap(
        long,
        help = "Path to the wallet fedimint client",
        env = "WALLET_FEDERATION_DB_PATH"
    )]
    pub wallet_federation_db_path: PathBuf,

    #[clap(long, help = "Cache capacity", value_parser = clap::value_parser!(NonZeroUsize), default_value = "5")]
    pub wallet_federation_cache_capacity: NonZeroUsize,

    #[clap(
        long,
        help = "This is the 'user' that receives the payments",
        env = "INTERNAL_USER_ID"
    )]
    pub internal_user_id: ChatUserId,

    #[arg(long, help = "Enable internal invoice generation", action)]
    pub allow_internal_invoice: bool,

    #[arg(
        long,
        help = "Enable staging/test endpoints (DO NOT USE IN PRODUCTION)",
        env = "STAGING"
    )]
    pub staging: bool,

    #[clap(flatten)]
    pub pg: PgParams,
}
