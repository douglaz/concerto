use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

pub mod common;
pub mod nostr_tests;
pub mod dkg_tests;
pub mod federation_tests;
pub mod simple_test;

/// Concerto Integration Tests
#[derive(Parser, Clone)]
#[command(version, about)]
struct Opts {
    #[clap(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand, Clone)]
enum CliCommand {
    /// Run the integration tests
    Run(RunArgs),
}

#[derive(Parser, Clone)]
struct RunArgs {
    // PostgreSQL configuration
    #[clap(long, env = "PGHOST", default_value = "localhost")]
    pub pghost: String,
    
    #[clap(long, env = "PGPORT", default_value = "15432")]
    pub pgport: u16,
    
    #[clap(long, env = "PGUSER", default_value = "postgres")]
    pub pguser: String,
    
    #[clap(long, env = "PGPASSWORD", default_value = "postgres")]
    pub pgpassword: String,
    
    #[clap(long, env = "PGDATABASE", default_value = "concerto_integration_tests")]
    pub pgdatabase: String,
    
    // Kubernetes configuration  
    #[clap(long, env = "KUBECONFIG", help = "Path to kubeconfig file")]
    kubeconfig: Option<String>,
    
    // Optional pre-built images for faster testing
    #[clap(long, env = "GUARDIANITO_IMAGE", help = "Pre-built guardianito image")]
    guardianito_image: Option<String>,
    
    #[clap(long, env = "FELAAS_IMAGE", help = "Pre-built felaas image")]
    felaas_image: Option<String>,
    
    #[clap(long, env = "NOSTR_RELAY_IMAGE", default_value = "scsibug/nostr-rs-relay:latest")]
    nostr_relay_image: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    initialize_logging();
    
    let opts = Opts::parse();
    match opts.command {
        CliCommand::Run(args) => {
            run_integration_tests(args).await?;
        }
    }
    Ok(())
}

async fn run_integration_tests(args: RunArgs) -> Result<()> {
    info!("Starting Concerto integration tests");
    info!("Working directory: {}", std::env::current_dir()?.display());
    
    // Connect to Kubernetes cluster
    info!("Connecting to Kubernetes cluster");
    let kube_client = common::connect_to_k8s(args.kubeconfig.as_deref()).await?;
    
    // Build and load container images
    info!("Preparing container images");
    let guardianito_image = if let Some(img) = args.guardianito_image {
        info!("Using pre-built guardianito image: {}", img);
        img
    } else {
        info!("Building guardianito image");
        common::build_and_load_guardianito_image().await?
    };
    
    let felaas_image = if let Some(img) = args.felaas_image {
        info!("Using pre-built felaas image: {}", img);
        img  
    } else {
        info!("Building felaas image");
        common::build_and_load_felaas_image().await?
    };
    
    // Deploy test infrastructure
    info!("Deploying test infrastructure");
    common::deploy_postgres_if_needed(&kube_client).await?;
    common::deploy_nostr_relay(&kube_client, &args.nostr_relay_image).await?;
    
    // Wait for services to be ready
    info!("Waiting for services to be ready");
    common::wait_for_postgres(&kube_client).await?;
    common::wait_for_nostr_relay(&kube_client).await?;
    
    // Setup test environment configuration
    let env_conf = common::EnvConf {
        nostr_relay_url: "ws://nostr-relay.default.svc.cluster.local:8008".to_string(),
        guardianito_image: guardianito_image.clone(),
        felaas_image: felaas_image.clone(),
    };
    
    // Create PostgreSQL connection parameters
    let pg_params = common::PgParams {
        pghost: args.pghost,
        pgport: args.pgport,
        pguser: args.pguser,
        pgpassword: args.pgpassword,
        pgdatabase: args.pgdatabase,
        pgschema: "public".to_string(), // Will be overridden per test
    };
    
    // Connect to PostgreSQL
    info!("Connecting to PostgreSQL");
    let pg_pool = common::connect_to_postgres(&pg_params).await?;
    
    // Run all tests in sequence
    info!("");
    info!("=== Running Nostr Coordination Tests ===");
    nostr_tests::test_nostr_relay_connectivity(&kube_client, &env_conf).await?;
    nostr_tests::test_multi_guardian_messaging(&kube_client, &env_conf, &pg_pool).await?;
    
    info!("");
    info!("=== Running DKG Integration Tests ===");
    dkg_tests::test_dkg_with_three_guardians(&kube_client, &env_conf, &pg_pool).await?;
    dkg_tests::test_dkg_setup_code_exchange(&kube_client, &env_conf, &pg_pool).await?;
    
    info!("");
    info!("=== Running Federation Lifecycle Tests ===");
    federation_tests::test_complete_federation_formation(&kube_client, &env_conf, &pg_pool).await?;
    federation_tests::test_federation_config_updates(&kube_client, &env_conf, &pg_pool).await?;
    federation_tests::test_guardian_slot_allocation(&kube_client, &env_conf, &pg_pool).await?;
    
    info!("");
    info!("✅ All integration tests passed successfully!");
    Ok(())
}

fn initialize_logging() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}