use std::net::SocketAddr;

use anyhow::Result;
use axum::Router;
use axum::routing::get;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::launch::configuration::db::FederationLauncherDB;
use crate::og_registry::db::OgRegistryDB;
use crate::wallet::core::FelaasWallet;
use crate::{ApiArgs, PgPool};

/// Health check handler
async fn health_check() -> &'static str {
    "Ok"
}

pub async fn run_api(args: ApiArgs) -> Result<()> {
    info!(?args, "Starting...");
    let wallet = crate::wallet::core::FelaasWallet::new(
        args.wallet_federation_invite_code,
        args.wallet_federation_db_path,
        args.internal_user_id,
        args.wallet_federation_cache_capacity,
    )
    .await?;
    info!("Creating pool...");
    let pool: PgPool = crate::common::create_pg_pool(&args.pg).await?;
    let subscription_db = crate::launch::subscription::db::SubscriptionDB::new(
        pool.clone(),
        args.pg.pgschema.clone(),
    );
    subscription_db.create_table_if_not_exists().await?;
    let launcher_db = FederationLauncherDB::new(pool.clone(), args.pg.pgschema.clone());
    launcher_db.create_table_if_not_exists().await?;
    let og_registry_db = OgRegistryDB::new(pool.clone(), args.pg.pgschema.clone());
    og_registry_db.create_table_if_not_exists().await?;
    // Launch subscription daemon
    info!("Launching subscription daemon...");
    tokio::spawn({
        let wallet = wallet.clone();
        let schema = args.pg.pgschema.clone();
        let pool = pool.clone();
        async move {
            crate::subscription_daemon::run_standard_daemon(schema, pool, &wallet).await;
        }
    });
    info!("Starting API...");
    run_server(
        wallet,
        subscription_db,
        launcher_db,
        og_registry_db,
        args.bind,
        args.allow_internal_invoice,
        args.staging,
    )
    .await
}

/// Sets up and runs the Axum server.
#[allow(clippy::too_many_arguments)]
pub async fn run_server(
    wallet: FelaasWallet,
    subscription_db: crate::launch::subscription::db::SubscriptionDB,
    launcher_db: FederationLauncherDB,
    og_registry_db: OgRegistryDB,
    addr: SocketAddr,
    allow_internal_invoice: bool,
    staging: bool,
) -> Result<()> {
    let state = AppState {
        wallet,
        subscription_db,
        launcher_db,
        og_registry_db,
        allow_internal_invoice,
        staging,
    };

    // Build our router with the routes.
    let app = Router::new()
        .route("/health", get(health_check))
        .nest("/wallet", crate::wallet::api::build_routes())
        .nest(
            "/subscription",
            crate::launch::subscription::api::build_routes(),
        )
        .nest(
            "/configuration",
            crate::launch::configuration::api::build_routes(),
        )
        .nest("/og-registry", crate::og_registry::api::build_routes())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!(%addr, "Listening");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Clone)]
pub struct AppState {
    pub wallet: FelaasWallet,
    pub subscription_db: crate::launch::subscription::db::SubscriptionDB,
    pub launcher_db: FederationLauncherDB,
    pub og_registry_db: OgRegistryDB,
    pub allow_internal_invoice: bool,
    pub staging: bool,
}
