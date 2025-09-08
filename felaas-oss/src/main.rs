use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use felaas_oss::{
    ApiArgs, PgParams, api, create_pg_pool, federation_launcher_daemon, guardian_launcher_tng,
    initialize_logging,
};
use secrecy::SecretString;
use tracing::info;
use url::Url;

#[derive(Parser, Clone)]
#[command(name = "felaas-oss")]
#[command(about = "Open Source Fedimint as a Service - Bitcoin-only subscriptions")]
#[command(version)]
struct Opts {
    #[clap(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand, Clone)]
enum CliCommand {
    #[command(about = "Run the FeLaaS API server")]
    Api(ApiArgs),

    #[command(about = "Launch Fedimint guardians on Kubernetes")]
    Launcher(guardian_launcher_tng::GuardianLauncherCmd),

    #[command(
        about = "Run the federation launcher daemon that monitors and provisions federations"
    )]
    FederationLauncherDaemon(federation_launcher_daemon::FederationLauncherDaemonArgs),
}

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub name: String,
    pub address: Url,
    pub password: Option<SecretString>,
}

impl std::str::FromStr for GatewayConfig {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = s.split(",").collect();
        match parts[..] {
            [name, address] => Ok(GatewayConfig {
                name: name.into(),
                address: address.parse()?,
                password: None,
            }),
            [name, address, password] => Ok(GatewayConfig {
                name: name.into(),
                address: address.parse()?,
                password: Some(SecretString::new(password.into())),
            }),
            _ => {
                bail!("Invalid gateway configuration, should use '<name>,<address>[,<password>]'")
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    initialize_logging();
    let opts = Opts::parse();
    match opts.command {
        CliCommand::Api(api_cmd) => {
            api::run_api(api_cmd).await?;
        }
        CliCommand::Launcher(guardian_cmd) => {
            guardian_launcher_tng::run_guardian_launcher(guardian_cmd).await?;
        }
        CliCommand::FederationLauncherDaemon(daemon_args) => {
            let pool = create_pg_pool(&daemon_args.pg).await?;
            federation_launcher_daemon::run_federation_launcher_daemon(daemon_args, pool).await?;
        }
    };
    info!("Done");
    Ok(())
}
