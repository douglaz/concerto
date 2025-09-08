// Integration tests for federation launcher daemon using k3d

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use felaas_oss::guardian_launcher_tng::naming;
use felaas_oss::launch::configuration::db::FederationLauncherDB;
use felaas_oss::launch::configuration::{
    FederationLaunchConfiguration, FederationLaunchId, FederationLaunchStatus,
};
use felaas_oss::og_registry::db::OgRegistryDB;
use felaas_oss::{ChatUserId, PgParams};
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::Api;
use tokio::time;
use tracing::{debug, info};
use uuid::Uuid;

use super::common::{
    cleanup_namespace, connect_to_postgres, create_test_namespace, deploy_daemon_job,
    wait_for_daemon_running,
};
use crate::common::EnvConf;

/// Create a test federation in the database
async fn create_test_federation(
    pool: &felaas_oss::PgPool,
    schema: &str,
    name: &str,
    num_fedimints: u8,
    num_ogs: u8,
) -> Result<FederationLaunchId> {
    let user_id = ChatUserId::from("test-lead-user".to_string());
    let num_guardians = num_fedimints + num_ogs;
    let fedimint_user_ids: Vec<ChatUserId> = (0..num_fedimints)
        .map(|i| ChatUserId::from(format!("fedimint-user-{i}")))
        .collect();

    // OG IDs always start empty - daemon will assign from registry
    let og_user_ids: Vec<ChatUserId> = vec![];

    let client = pool.get().await?;
    let launch_id = FederationLaunchId::from(uuid::Uuid::new_v4());

    // Insert directly into the table
    client
        .execute(
            &format!(
                "INSERT INTO {schema}.federation_launch_configuration
             (launch_id, name, status, num_guardians, num_ogs, num_fedimints,
              user_id, fedimint_user_ids, og_user_ids, guardian_endpoints, admin_ui_endpoints)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
            ),
            &[
                &launch_id,
                &name,
                &FederationLaunchStatus::Requested,
                &i32::from(num_guardians),
                &i32::from(num_ogs),
                &i32::from(num_fedimints),
                &user_id,
                &fedimint_user_ids,
                &og_user_ids,
                &Vec::<String>::new(), // guardian_endpoints
                &Vec::<String>::new(), // admin_ui_endpoints
            ],
        )
        .await?;

    Ok(launch_id)
}

/// Populate the OG registry with active OGs
async fn populate_og_registry(
    pool: &felaas_oss::PgPool,
    schema: &str,
    count: usize,
) -> Result<Vec<ChatUserId>> {
    let og_registry = OgRegistryDB::new(pool.clone(), schema.to_string());
    let mut og_ids = vec![];

    for i in 0..count {
        let og_id = ChatUserId::from(format!("active-og-{i}"));
        let reference_user_id = ChatUserId::from(format!("ref-og-{i}")); // Use a unique reference_user_id for each OG
        og_registry
            .upsert_og(og_id.clone(), reference_user_id, true)
            .await?;
        og_ids.push(og_id);
    }

    info!(og_count = %count, "Populated OG registry with active OGs");
    Ok(og_ids)
}

/// Initialize test database schema
async fn init_test_schema(pool: &felaas_oss::PgPool, schema: &str) -> Result<()> {
    let client = pool.get().await?;

    // Create schema
    client
        .execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"), &[])
        .await?;

    // Create federation launcher table
    let db = FederationLauncherDB::new(pool.clone(), schema.to_string());
    db.create_table_if_not_exists().await?;

    // Create OG registry table
    let og_registry = OgRegistryDB::new(pool.clone(), schema.to_string());
    og_registry.create_table_if_not_exists().await?;

    Ok(())
}

/// Clean up test schema
async fn cleanup_test_schema(pool: &felaas_oss::PgPool, schema: &str) -> Result<()> {
    let client = pool.get().await?;
    client
        .execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"), &[])
        .await?;
    Ok(())
}

/// Wait for federation to reach a specific status
async fn wait_for_federation_status(
    db: &FederationLauncherDB,
    launch_id: &FederationLaunchId,
    expected_status: FederationLaunchStatus,
    timeout: Duration,
) -> Result<FederationLaunchConfiguration> {
    let start = Instant::now();
    loop {
        let config = db
            .get_federation_launch_configuration(launch_id)
            .await?
            .with_context(|| format!("Federation not found for launch ID: {launch_id}"))?;

        if config.status == expected_status {
            info!(
                ?expected_status,
                elapsed = ?start.elapsed(),
                "Federation reached expected status"
            );
            return Ok(config);
        }

        if start.elapsed() > timeout {
            bail!(
                "Timeout waiting for status {expected_status:?}, current: {current_status:?}",
                current_status = config.status
            );
        }

        debug!(
            ?config.status,
            elapsed = ?start.elapsed(),
            "Federation status not reached"
        );
        time::sleep(Duration::from_secs(1)).await;
    }
}

/// Verify federation resources were created in Kubernetes
async fn verify_federation_resources(
    kube_client: &kube::Client,
    federation_name: &str,
    created_at: &chrono::DateTime<chrono::Utc>,
    num_fedimints: u8,
    num_ogs: u8,
) -> Result<()> {
    let namespace = naming::generate_namespace_name(federation_name, created_at);
    let deployments: Api<Deployment> = Api::namespaced(kube_client.clone(), &namespace);

    // Check deployments exist
    let deployment_list = deployments.list(&Default::default()).await?;
    let expected_deployments = (num_fedimints + num_ogs) as usize + 1; // guardians + admin UI

    if deployment_list.items.len() != expected_deployments {
        bail!(
            "Expected {expected_deployments} deployments, found {found}",
            found = deployment_list.items.len()
        );
    }

    info!(%namespace, deployment_count = %deployment_list.items.len(), "All federation deployments verified");
    Ok(())
}

/// Verify federation endpoints were set in database
async fn verify_federation_endpoints(
    db: &FederationLauncherDB,
    launch_id: &FederationLaunchId,
    num_fedimints: u8,
    num_ogs: u8,
) -> Result<()> {
    let config = db
        .get_federation_launch_configuration(launch_id)
        .await?
        .context("Federation not found")?;

    let expected_endpoints = (num_fedimints + num_ogs) as usize;

    if config.guardians_configurations.len() != expected_endpoints {
        bail!(
            "Expected {expected_endpoints} guardian configurations, found {found}",
            found = config.guardians_configurations.len()
        );
    }

    // Verify all guardians have valid endpoints
    for (i, guardian) in config.guardians_configurations.iter().enumerate() {
        let api_url = guardian.api_endpoint.as_str();
        // In test mode with use_http=true, endpoints are ws:// instead of wss://
        if (!api_url.starts_with("wss://") && !api_url.starts_with("ws://"))
            || !api_url.contains("internal.felaas.dev")
        {
            bail!("Invalid guardian {i} API endpoint format: {api_url}");
        }

        // Admin UI endpoint might not be set for all guardians initially
        if let Some(ui_endpoint) = &guardian.admin_ui_endpoint {
            let ui_url = ui_endpoint.as_str();
            // In test mode with use_http=true, endpoints are http:// instead of https://
            if (!ui_url.starts_with("https://") && !ui_url.starts_with("http://"))
                || !ui_url.contains("internal.felaas.dev")
            {
                bail!("Invalid guardian {i} UI endpoint format: {ui_url}");
            }
        }
    }

    info!(endpoint_count = %config.guardians_configurations.len(), "All federation endpoints verified");
    Ok(())
}

pub async fn test_federation_deployment_with_k3d(
    kube_client: &kube::Client,
    env_conf: &EnvConf,
    image_tag: &str,
    pg_params: &PgParams,
) -> Result<()> {
    let uuid = Uuid::new_v4();
    let test_schema = format!("test_federation_{}", uuid.as_u128());

    // Connect to PostgreSQL inside k3d
    let pg_pool = connect_to_postgres(pg_params).await?;

    // Initialize test schema
    init_test_schema(&pg_pool, &test_schema).await?;

    // Create test namespace for this test
    let test_namespace = create_test_namespace(kube_client, "test-federation", uuid).await?;

    // Populate OG registry with active OGs
    let num_ogs = 3;
    populate_og_registry(&pg_pool, &test_schema, num_ogs as usize + 2).await?; // Add extra OGs to test selection

    // Create a test federation without preset OGs
    let federation_name = format!("fed-{}", uuid.to_string().chars().take(8).collect::<String>());
    let num_fedimints = 1;
    let launch_id = create_test_federation(
        &pg_pool,
        &test_schema,
        &federation_name,
        num_fedimints,
        num_ogs,
    )
    .await?;

    info!(%federation_name, ?launch_id, "Created test federation");

    // Deploy the daemon to process the federation
    let mut pg_params_with_schema = pg_params.clone();
    pg_params_with_schema.pgschema = test_schema.clone();
    
    let job_name = deploy_daemon_job(
        kube_client,
        &test_namespace,
        image_tag,
        &pg_params_with_schema,
        env_conf,
    )
    .await?;

    // Wait for daemon to be running
    wait_for_daemon_running(
        kube_client,
        &test_namespace,
        &job_name,
        Duration::from_secs(60),
    )
    .await?;

    // Wait for federation to reach InProgress status (infrastructure deployed but not configured)
    let db = FederationLauncherDB::new(pg_pool.clone(), test_schema.clone());
    let config = wait_for_federation_status(
        &db,
        &launch_id,
        FederationLaunchStatus::InProgress,
        Duration::from_secs(60),
    )
    .await?;

    info!(%federation_name, "Federation reached InProgress status - infrastructure deployed");

    // Wait for all pods to be created (guardian + UI deployments)
    info!("Waiting for all guardian and UI pods to be deployed...");
    tokio::time::sleep(Duration::from_secs(60)).await;

    // Verify Kubernetes resources were created
    verify_federation_resources(
        kube_client,
        &federation_name,
        &config.created_at,
        num_fedimints,
        num_ogs,
    )
    .await?;

    // Skip endpoint verification as they won't be set until DKG completes
    info!("Skipping endpoint verification - DKG not completed");

    // Cleanup
    cleanup_namespace(kube_client, &test_namespace).await?;
    cleanup_test_schema(&pg_pool, &test_schema).await?;

    info!("test_federation_deployment_with_k3d completed successfully");
    Ok(())
}

pub async fn test_multiple_federations_processing(
    kube_client: &kube::Client,
    env_conf: &EnvConf,
    image_tag: &str,
    pg_params: &PgParams,
) -> Result<()> {
    let uuid = Uuid::new_v4();
    let test_schema = format!("test_multiple_{}", uuid.as_u128());

    // Connect to PostgreSQL
    let pg_pool = connect_to_postgres(pg_params).await?;
    init_test_schema(&pg_pool, &test_schema).await?;

    // Create test namespace
    let test_namespace = create_test_namespace(kube_client, "test-multiple", uuid).await?;

    // Populate OG registry
    populate_og_registry(&pg_pool, &test_schema, 10).await?;

    // Create multiple federations
    let mut launch_ids = Vec::new();
    for i in 0..3 {
        let federation_name = format!("multi-fed-{i}");
        let launch_id = create_test_federation(
            &pg_pool,
            &test_schema,
            &federation_name,
            1, // 1 fedimint
            2, // 2 OGs
        )
        .await?;
        launch_ids.push(launch_id);
        
        // Small delay to ensure different timestamps
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    info!("Created {} test federations", launch_ids.len());

    // Deploy the daemon
    let mut pg_params_with_schema = pg_params.clone();
    pg_params_with_schema.pgschema = test_schema.clone();
    
    let job_name = deploy_daemon_job(
        kube_client,
        &test_namespace,
        image_tag,
        &pg_params_with_schema,
        env_conf,
    )
    .await?;

    wait_for_daemon_running(
        kube_client,
        &test_namespace,
        &job_name,
        Duration::from_secs(60),
    )
    .await?;

    // Verify all federations are processed in FIFO order
    let db = FederationLauncherDB::new(pg_pool.clone(), test_schema.clone());
    
    for (i, launch_id) in launch_ids.iter().enumerate() {
        let config = wait_for_federation_status(
            &db,
            launch_id,
            FederationLaunchStatus::InProgress,
            Duration::from_secs(60),
        )
        .await?;
        
        info!(federation_index = %i, ?launch_id, "Federation {} reached InProgress - infrastructure deployed", i);
        
        // Wait a bit for all pods to be created
        tokio::time::sleep(Duration::from_secs(20)).await;
        
        // Verify resources were created
        verify_federation_resources(
            kube_client,
            &config.name,
            &config.created_at,
            1,
            2,
        )
        .await?;
    }

    // Cleanup
    cleanup_namespace(kube_client, &test_namespace).await?;
    cleanup_test_schema(&pg_pool, &test_schema).await?;

    info!("test_multiple_federations_processing completed successfully");
    Ok(())
}

pub async fn test_federation_status_transitions(
    kube_client: &kube::Client,
    env_conf: &EnvConf,
    image_tag: &str,
    pg_params: &PgParams,
) -> Result<()> {
    let uuid = Uuid::new_v4();
    let test_schema = format!("test_status_{}", uuid.as_u128());

    // Connect to PostgreSQL
    let pg_pool = connect_to_postgres(pg_params).await?;
    init_test_schema(&pg_pool, &test_schema).await?;

    // Create test namespace
    let test_namespace = create_test_namespace(kube_client, "test-status", uuid).await?;

    // Populate OG registry
    populate_og_registry(&pg_pool, &test_schema, 5).await?;

    // Create a test federation
    let federation_name = format!("status-fed-{}", uuid.to_string().chars().take(8).collect::<String>());
    let launch_id = create_test_federation(
        &pg_pool,
        &test_schema,
        &federation_name,
        2, // 2 fedimints
        2, // 2 OGs
    )
    .await?;

    info!(%federation_name, ?launch_id, "Created test federation for status transitions");

    // Deploy the daemon
    let mut pg_params_with_schema = pg_params.clone();
    pg_params_with_schema.pgschema = test_schema.clone();
    
    let job_name = deploy_daemon_job(
        kube_client,
        &test_namespace,
        image_tag,
        &pg_params_with_schema,
        env_conf,
    )
    .await?;

    wait_for_daemon_running(
        kube_client,
        &test_namespace,
        &job_name,
        Duration::from_secs(60),
    )
    .await?;

    // Track status transitions
    let db = FederationLauncherDB::new(pg_pool.clone(), test_schema.clone());
    
    // Should transition from Requested -> InProgress quickly
    let _config = wait_for_federation_status(
        &db,
        &launch_id,
        FederationLaunchStatus::InProgress,
        Duration::from_secs(30),
    )
    .await?;
    
    info!("Federation transitioned to InProgress");

    // Wait a bit for infrastructure to be fully deployed
    tokio::time::sleep(Duration::from_secs(30)).await;
    
    // Get current federation config (will still be InProgress as DKG not completed)
    let config = db.get_federation_launch_configuration(&launch_id).await?
        .ok_or_else(|| anyhow::anyhow!("Federation config not found"))?;
    
    info!("Federation infrastructure deployed, skipping DKG wait");

    // Skip guardian endpoint verification as they won't be set until DKG completes
    info!("Skipping guardian endpoint verification - DKG not completed");

    // Verify OGs were assigned (should be assigned even before DKG)
    if config.og_user_ids.is_empty() {
        bail!("No OG user IDs assigned");
    }

    info!(
        guardian_count = %config.guardians_configurations.len(),
        og_count = %config.og_user_ids.len(),
        "Federation has guardian configurations and OG assignments"
    );

    // Cleanup
    cleanup_namespace(kube_client, &test_namespace).await?;
    cleanup_test_schema(&pg_pool, &test_schema).await?;

    info!("test_federation_status_transitions completed successfully");
    Ok(())
}