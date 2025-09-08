use clap::{Parser, Subcommand};

pub mod common;
mod k3d_test;

use secrecy::SecretString;
use tracing::info;

use crate::common::{
    EnvConf, build_and_load_daemon_image, configure_coredns, connect_to_k8s,
    create_storage_classes, deploy_daemon_rbac, deploy_postgres_if_needed,
    install_nginx_ingress_controller, label_nodes_with_zone, load_fedimint_images,
    wait_for_postgres,
};

#[derive(Parser, Clone)]
#[command(version)]
struct Opts {
    #[clap(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand, Clone)]
enum CliCommand {
    #[command(about = "Run the tests")]
    Run(RunArgs),
}

#[derive(Parser, Clone)]
#[command(version)]
struct RunArgs {
    // PostgreSQL configuration
    #[clap(flatten)]
    pub pg: felaas_oss::PgParams,

    // Kubernetes configuration
    #[clap(long, env = "KUBECONFIG", help = "Path to kubeconfig file")]
    kubeconfig: Option<String>,

    // Image configuration
    #[clap(
        long,
        default_value = "fedibtc/fedi-fedimintd:v0.7.2-fedi1-deployment1"
    )]
    fedimint_image_name: String,
    #[clap(long, default_value = "fedibtc/fedimint-ui:0.7.0")]
    ui_image_name: String,

    // Optional pre-built image for testing
    #[clap(
        long,
        env = "FELAAS_TEST_IMAGE_TAG",
        help = "Pre-built felaas image tag to use"
    )]
    felaas_image_tag: Option<String>,

    // Test configuration
    #[clap(long, default_value = "internal.felaas.dev")]
    fedimint_external_domain: String,
    #[clap(long, default_value = "internal.felaas.dev")]
    ui_external_domain: String,
    #[clap(long, default_value = "test-az")]
    availability_zone: String,
    #[clap(long, default_value = "testpass")]
    bitcoin_rpc_password: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging only if not already initialized
    use tracing_subscriber::prelude::*;
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .try_init();
    let opts = Opts::parse();
    match opts.command {
        CliCommand::Run(run_args) => {
            run_integration_tests(run_args).await?;
        }
    }
    Ok(())
}

async fn run_integration_tests(run_args: RunArgs) -> anyhow::Result<()> {
    eprintln!(
        "Starting integration tests at cwd: {cwd}",
        cwd = std::env::current_dir()?.display()
    );

    tracing::info!("Setting up k3d test environment");

    let env_conf = EnvConf {
        fedimint_external_domain: run_args.fedimint_external_domain.clone(),
        ui_external_domain: run_args.ui_external_domain.clone(),
        az: run_args.availability_zone.clone(),
        image_name: run_args.fedimint_image_name,
        ui_image_name: run_args.ui_image_name,
        bitcoin_rpc_password: SecretString::from(run_args.bitcoin_rpc_password.clone()),
    };

    let kube_client = connect_to_k8s(run_args.kubeconfig.as_deref()).await?;

    let felaas_image_tag = run_args.felaas_image_tag.clone();
    let build_and_load_task =
        tokio::spawn(async move { build_and_load_daemon_image(felaas_image_tag).await });

    label_nodes_with_zone(&kube_client, &env_conf.az).await?;

    create_storage_classes(&kube_client).await?;

    deploy_postgres_if_needed(&kube_client).await?;

    wait_for_postgres(&kube_client).await?;

    install_nginx_ingress_controller(&kube_client).await?;

    configure_coredns(&kube_client, run_args.kubeconfig.as_deref()).await?;

    deploy_daemon_rbac(&kube_client).await?;

    // Load Fedimint images into k3d
    load_fedimint_images(&env_conf.image_name, &env_conf.ui_image_name).await?;

    let image_tag = build_and_load_task.await??;

    info!(%image_tag, "Test environment ready, will start tests");

    // Run tests with the loaded image tag
    k3d_test::test_federation_deployment_with_k3d(
        &kube_client,
        &env_conf,
        &image_tag,
        &run_args.pg,
    )
    .await?;

    k3d_test::test_multiple_federations_processing(
        &kube_client,
        &env_conf,
        &image_tag,
        &run_args.pg,
    )
    .await?;

    k3d_test::test_federation_status_transitions(&kube_client, &env_conf, &image_tag, &run_args.pg)
        .await?;

    info!("All integration tests completed successfully!");

    Ok(())
}
