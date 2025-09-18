mod provider;
mod economics;
mod deployment;
mod api;
mod database;

use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;
use tracing::{info, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "felaas")]
#[command(about = "Concerto FeLaaS Provider - Economically aware federation hosting", long_about = None)]
struct Cli {
    /// Provider's Nostr public key (npub or hex)
    #[arg(long, env = "PROVIDER_NPUB")]
    provider_npub: String,
    
    /// Provider's Nostr private key (nsec or hex)
    #[arg(long, env = "PROVIDER_NSEC")]
    provider_nsec: String,
    
    /// Provider name
    #[arg(long, env = "PROVIDER_NAME")]
    provider_name: String,
    
    /// Database URL (PostgreSQL)
    #[arg(long, env = "DATABASE_URL", default_value = "postgres://localhost/felaas")]
    database_url: String,
    
    /// API listen address
    #[arg(long, env = "API_ADDR", default_value = "0.0.0.0:8080")]
    api_addr: String,
    
    /// Deployment backend
    #[arg(long, env = "DEPLOYMENT_BACKEND", default_value = "docker")]
    deployment_backend: DeploymentBackend,
    
    /// Minimum subscription tier
    #[arg(long, env = "MIN_SUBSCRIPTION_TIER", default_value = "basic")]
    min_subscription_tier: String,
    
    /// Base slot price in satoshis
    #[arg(long, env = "BASE_SLOT_PRICE_SATS", default_value = "100000")]
    base_slot_price_sats: u64,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Debug)]
enum DeploymentBackend {
    Docker,
    Kubernetes,
}

impl std::str::FromStr for DeploymentBackend {
    type Err = String;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "docker" => Ok(DeploymentBackend::Docker),
            "kubernetes" | "k8s" => Ok(DeploymentBackend::Kubernetes),
            _ => Err(format!("Unknown deployment backend: {}", s)),
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Run provider in daemon mode
    Daemon,
    
    /// Initialize database
    InitDb,
    
    /// Show provider info
    Info,
    
    /// Manage slots
    Slot {
        #[command(subcommand)]
        action: SlotCommands,
    },
    
    /// Economic reports
    Economics {
        #[command(subcommand)]
        action: EconomicsCommands,
    },
}

#[derive(Subcommand)]
enum SlotCommands {
    /// List all slots
    List,
    
    /// Show slot details
    Show {
        slot_id: String,
    },
    
    /// Manually allocate a slot
    Allocate {
        federation_id: String,
        guardian_npub: String,
    },
    
    /// Release a slot
    Release {
        slot_id: String,
    },
}

#[derive(Subcommand)]
enum EconomicsCommands {
    /// Show current pricing
    Pricing,
    
    /// Show revenue report
    Revenue {
        #[arg(long, default_value = "30")]
        days: u32,
    },
    
    /// Show utilization report
    Utilization,
    
    /// Update pricing model
    UpdatePricing {
        #[arg(long)]
        base_price_sats: Option<u64>,
        #[arg(long)]
        high_demand_multiplier: Option<f32>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "felaas=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    
    // Parse provider keys
    let provider_keys = if cli.provider_nsec.starts_with("nsec") {
        nostr_sdk::Keys::from_str(&cli.provider_nsec)?
    } else {
        let secret_key = nostr_sdk::secp256k1::SecretKey::from_str(&cli.provider_nsec)?;
        nostr_sdk::Keys::new(secret_key)
    };
    
    info!("FeLaaS Provider starting: {}", cli.provider_name);
    info!("Provider public key: {}", provider_keys.public_key().to_bech32()?);
    
    // Connect to database
    let db_pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cli.database_url)
        .await?;
    
    // Execute command
    match cli.command {
        Commands::Daemon => {
            info!("Starting FeLaaS daemon mode");
            run_daemon(cli, provider_keys, db_pool).await?;
        }
        Commands::InitDb => {
            info!("Initializing database");
            database::init_database(&db_pool).await?;
            info!("Database initialized successfully");
        }
        Commands::Info => {
            show_provider_info(&cli, &provider_keys).await?;
        }
        Commands::Slot { action } => {
            handle_slot_command(action, &db_pool).await?;
        }
        Commands::Economics { action } => {
            handle_economics_command(action, &db_pool).await?;
        }
    }
    
    Ok(())
}

async fn run_daemon(
    config: Cli,
    keys: nostr_sdk::Keys,
    db_pool: sqlx::PgPool,
) -> anyhow::Result<()> {
    // Create provider instance
    let provider = provider::FeLaaSProvider::new(
        keys,
        config.provider_name.clone(),
        vec!["US".to_string()], // TODO: Make configurable
        config.min_subscription_tier.clone(),
        config.base_slot_price_sats,
        db_pool.clone(),
    );
    
    // Create deployment backend
    let deployment = match config.deployment_backend {
        DeploymentBackend::Docker => {
            deployment::DeploymentBackend::Docker(
                deployment::DockerBackend::new().await?
            )
        }
        DeploymentBackend::Kubernetes => {
            deployment::DeploymentBackend::Kubernetes(
                deployment::KubernetesBackend::new().await?
            )
        }
    };
    
    // Start API server
    let api_state = api::ApiState {
        provider: provider.clone(),
        deployment,
        db_pool,
    };
    
    let app = api::create_app(api_state);
    
    let listener = tokio::net::TcpListener::bind(&config.api_addr).await?;
    info!("API server listening on {}", config.api_addr);
    
    // Run API server and provider tasks concurrently
    tokio::select! {
        result = axum::serve(listener, app) => {
            if let Err(e) = result {
                error!("API server error: {}", e);
            }
        }
        result = provider.run_background_tasks() => {
            if let Err(e) = result {
                error!("Provider background tasks error: {}", e);
            }
        }
    }
    
    Ok(())
}

async fn show_provider_info(
    config: &Cli,
    keys: &nostr_sdk::Keys,
) -> anyhow::Result<()> {
    println!("FeLaaS Provider Information");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Name: {}", config.provider_name);
    println!("Public Key: {}", keys.public_key().to_bech32()?);
    println!("API Address: {}", config.api_addr);
    println!("Deployment: {:?}", config.deployment_backend);
    println!("Min Tier: {}", config.min_subscription_tier);
    println!("Base Price: {} sats", config.base_slot_price_sats);
    Ok(())
}

async fn handle_slot_command(
    action: SlotCommands,
    db_pool: &sqlx::PgPool,
) -> anyhow::Result<()> {
    match action {
        SlotCommands::List => {
            let slots = database::list_slots(db_pool).await?;
            println!("Hosted slots: {}", slots.len());
            for slot in slots {
                println!("  {} - {:?}", slot.slot_id, slot.status);
            }
        }
        SlotCommands::Show { slot_id } => {
            let slot = database::get_slot(db_pool, &slot_id).await?;
            println!("Slot Details:");
            println!("  ID: {}", slot.slot_id);
            println!("  Guardian: {}", slot.guardian_npub);
            println!("  Federation: {}", slot.federation_id);
            println!("  Status: {}", slot.status);
            println!("  Allocated: {}", slot.allocated_at);
        }
        SlotCommands::Allocate { federation_id, guardian_npub } => {
            // TODO: Implement manual allocation
            println!("Manual allocation not yet implemented");
        }
        SlotCommands::Release { slot_id } => {
            database::release_slot(db_pool, &slot_id).await?;
            println!("Slot {} released", slot_id);
        }
    }
    Ok(())
}

async fn handle_economics_command(
    action: EconomicsCommands,
    db_pool: &sqlx::PgPool,
) -> anyhow::Result<()> {
    match action {
        EconomicsCommands::Pricing => {
            let pricing = database::get_current_pricing(db_pool).await?;
            println!("Current Pricing Model:");
            println!("  Base Slot Price: {} sats", pricing.base_slot_price_sats);
            println!("  CPU/hour: {} sats", pricing.cpu_per_core_hour_sats);
            println!("  Memory/GB/hour: {} sats", pricing.memory_per_gb_hour_sats);
            println!("  Storage/GB/month: {} sats", pricing.storage_per_gb_month_sats);
            println!("  Bandwidth/GB: {} sats", pricing.bandwidth_per_gb_sats);
        }
        EconomicsCommands::Revenue { days } => {
            let revenue = database::get_revenue_report(db_pool, days).await?;
            println!("Revenue Report ({} days):", days);
            println!("  Total Revenue: {} sats", revenue.total_revenue_sats);
            println!("  Total Costs: {} sats", revenue.total_costs_sats);
            println!("  Profit: {} sats", revenue.profit_sats);
            println!("  Margin: {:.2}%", revenue.profit_margin_percent);
        }
        EconomicsCommands::Utilization => {
            let util = database::get_utilization_report(db_pool).await?;
            println!("Utilization Report:");
            println!("  Total Slots: {}", util.total_slots);
            println!("  Active Slots: {}", util.active_slots);
            println!("  Utilization: {:.2}%", util.utilization_percent);
            println!("  Avg CPU Usage: {:.2}%", util.avg_cpu_percent);
            println!("  Avg Memory Usage: {:.2}%", util.avg_memory_percent);
        }
        EconomicsCommands::UpdatePricing { base_price_sats, high_demand_multiplier } => {
            if let Some(price) = base_price_sats {
                database::update_base_price(db_pool, price).await?;
                println!("Updated base price to {} sats", price);
            }
            if let Some(multiplier) = high_demand_multiplier {
                database::update_demand_multiplier(db_pool, multiplier).await?;
                println!("Updated high demand multiplier to {}", multiplier);
            }
        }
    }
    Ok(())
}