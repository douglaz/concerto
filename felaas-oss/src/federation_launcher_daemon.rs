use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
use fedimint_core::bitcoin::Network;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use secrecy::SecretString;
use thiserror::Error;
use tokio::time;
use tracing::{debug, error, info, warn};
use url::Url;
use urlencoding;

use crate::common::Endpoint;
use crate::fedimint_status_client::{verify_guardians_accessible, GuardianStatusError};
use crate::guardian_launcher_tng::{
    apply_guardian_deployment_objects, apply_guardian_ui_objects, naming,
    render_guardian_deployment_objects, render_guardian_ui_objects, GuardianDeploymentParameters,
    RenderGuardianUiArgs,
};
use crate::launch::configuration::db::FederationLauncherDB;
use crate::launch::configuration::{FederationLaunchConfiguration, FederationLaunchStatus};
use crate::og_registry::db::OgRegistryDB;
use crate::PgPool;

#[derive(Debug, Error)]
pub(crate) enum DeploymentError {
    #[error("Deployment timeout exceeded for {namespace}/{deployment_name} after {timeout_secs} seconds")]
    DeploymentTimeout {
        namespace: String,
        deployment_name: String,
        timeout_secs: u64,
    },

    #[error("Guardian verification failed")]
    GuardianVerification(#[from] GuardianStatusError),

    #[error("Kubernetes resource creation failed")]
    KubernetesApply(#[source] anyhow::Error),

    #[error("Failed to render deployment objects")]
    RenderFailed(#[source] anyhow::Error),

    #[error("Other deployment error")]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Args, Clone)]
pub struct FederationLauncherDaemonArgs {
    #[clap(flatten)]
    pub pg: crate::PgParams,

    #[clap(long, help = "External domain for Fedimint Api ingress")]
    pub fedimint_external_domain: String,

    #[clap(long, help = "External domain for Admin UI ingress")]
    pub ui_external_domain: String,

    #[clap(long, help = "Availability zone")]
    pub az: String,

    #[clap(long, help = "Guardian docker image")]
    pub image_name: String,

    #[clap(long, help = "Guardian UI docker image")]
    pub ui_image_name: String,

    #[clap(long, help = "Bitcoin RPC password", env = "BITCOIN_RPC_PASSWORD")]
    pub bitcoin_rpc_password: SecretString,

    #[clap(long, help = "Bitcoin Network", env = "BITCOIN_NETWORK")]
    pub bitcoin_network: Network,

    #[clap(long, help = "Polling interval in seconds", default_value = "10")]
    pub poll_interval_secs: u64,

    #[clap(
        long,
        help = "Timeout in seconds to wait for deployments to be ready",
        default_value = "3600"
    )]
    pub deployment_timeout_secs: u64,

    #[clap(
        long,
        help = "Interval in seconds between guardian readiness checks",
        default_value = "10"
    )]
    pub readiness_check_interval_secs: u64,

    #[clap(
        long,
        help = "Timeout in seconds for guardian readiness checks",
        default_value = "3600"
    )]
    pub readiness_check_timeout_secs: u64,

    #[clap(
        long,
        help = "Maximum retries for guardian readiness checks",
        default_value = "360"
    )]
    pub readiness_check_max_retries: u32,

    #[clap(
        long,
        help = "Use HTTP/WS instead of HTTPS/WSS (for testing environments without TLS)",
        default_value = "false"
    )]
    pub use_http: bool,

    #[clap(
        long,
        help = "Run in test mode with reduced resource requirements",
        default_value = "false"
    )]
    pub test_mode: bool,
}

/// Main daemon loop that processes federation launch requests
pub async fn run_federation_launcher_daemon(
    args: FederationLauncherDaemonArgs,
    pool: PgPool,
) -> Result<()> {
    info!(
        "Starting federation launcher daemon with test_mode={}",
        args.test_mode
    );

    let db: FederationLauncherDB =
        FederationLauncherDB::new(pool.clone(), args.pg.pgschema.clone());

    let og_registry_db: OgRegistryDB = OgRegistryDB::new(pool.clone(), args.pg.pgschema.clone());

    // Create kube client
    let kube_client = kube::Client::try_default().await?;

    loop {
        debug!("Processing next federation...");
        match process_next_federation(&args, &db, &og_registry_db, &kube_client).await {
            Ok(Some(federation)) => {
                info!(
                    federation_id = %federation.launch_id,
                    federation_name = %federation.name,
                    "Successfully processed federation"
                );
            }
            Ok(None) => {
                debug!("No pending federations to process");
                time::sleep(Duration::from_secs(args.poll_interval_secs)).await;
            }
            Err(e) => {
                error!(?e, "Error processing federation");
                time::sleep(Duration::from_secs(args.poll_interval_secs)).await;
            }
        }
    }
}

async fn process_next_federation(
    args: &FederationLauncherDaemonArgs,
    db: &FederationLauncherDB,
    og_registry_db: &OgRegistryDB,
    kube_client: &kube::Client,
) -> Result<Option<FederationLaunchConfiguration>> {
    // Get the oldest federation in Requested, InProgress, or InfrastructureReady
    // status InProgress federations might have failed previously and need retry
    // InfrastructureReady federations need OG assignment
    let mut federations = db
        .search_launch_configurations(&[], &[], &[FederationLaunchStatus::Requested], None)
        .await?;

    let in_progress_federations = db
        .search_launch_configurations(&[], &[], &[FederationLaunchStatus::InProgress], None)
        .await?;

    let infrastructure_ready_federations = db
        .search_launch_configurations(
            &[],
            &[],
            &[FederationLaunchStatus::InfrastructureReady],
            None,
        )
        .await?;

    federations.extend(in_progress_federations);
    federations.extend(infrastructure_ready_federations);

    // Sort by created_at to get FIFO order (oldest first)
    let federation = federations.into_iter().min_by_key(|f| f.created_at);

    let Some(federation) = federation else {
        return Ok(None);
    };

    info!(
        federation_id = %federation.launch_id,
        federation_name = %federation.name,
        status = ?federation.status,
        created_at = %federation.created_at,
        age_mins = (chrono::Utc::now() - federation.created_at).num_minutes(),
        "Processing federation launch request"
    );

    // Handle federation based on its current status
    match federation.status {
        FederationLaunchStatus::InfrastructureReady => {
            // Infrastructure is ready, try to assign OGs
            info!(
                federation_id = %federation.launch_id,
                "Federation infrastructure ready, attempting OG assignment"
            );
            return handle_og_assignment(db, og_registry_db, federation).await;
        }
        FederationLaunchStatus::Requested => {
            info!(
                federation_id = %federation.launch_id,
                "Transitioning federation from Requested to InProgress"
            );
            db.update_status(
                federation.launch_id.clone(),
                FederationLaunchStatus::InProgress,
            )
            .await
            .context("Failed to update status to InProgress")?;
        }
        FederationLaunchStatus::InProgress => {
            info!(
                federation_id = %federation.launch_id,
                "Retrying federation already in InProgress status"
            );
        }
        _ => {
            // Should not happen based on our query
            warn!(
                federation_id = %federation.launch_id,
                status = ?federation.status,
                "Unexpected federation status in process_next_federation"
            );
            return Ok(None);
        }
    }

    // Deploy guardians and collect endpoints
    match deploy_federation(args, &federation, kube_client).await {
        Ok((guardian_endpoints, admin_ui_endpoints)) => {
            // Convert string endpoints to Endpoint type
            let guardian_endpoints: Vec<Endpoint> = guardian_endpoints
                .into_iter()
                .map(Endpoint::from)
                .collect::<Vec<_>>();
            let admin_ui_endpoints: Vec<Endpoint> = admin_ui_endpoints
                .into_iter()
                .map(Endpoint::from)
                .collect::<Vec<_>>();

            // Update endpoints in database
            db.set_guardian_endpoint(
                federation.launch_id.clone(),
                guardian_endpoints,
                admin_ui_endpoints,
            )
            .await
            .context("Failed to update endpoints")?;

            // Update status to InfrastructureReady (infrastructure is deployed and
            // endpoints are set)
            db.update_status(
                federation.launch_id.clone(),
                FederationLaunchStatus::InfrastructureReady,
            )
            .await
            .context("Failed to update status to InfrastructureReady")?;

            info!(
                federation_id = %federation.launch_id,
                "Federation infrastructure deployed successfully, will attempt OG assignment"
            );

            // Now try to assign OGs
            handle_og_assignment(db, og_registry_db, federation).await
        }
        Err(e) => {
            let age_mins = (chrono::Utc::now() - federation.created_at).num_minutes();

            // Log specific error details based on the error type
            match &e {
                DeploymentError::DeploymentTimeout {
                    namespace,
                    deployment_name,
                    timeout_secs,
                } => {
                    error!(
                        federation_id = %federation.launch_id,
                        federation_name = %federation.name,
                        namespace,
                        deployment_name,
                        timeout_secs,
                        age_mins,
                        "Deployment timeout exceeded, will retry on next poll cycle"
                    );
                }
                DeploymentError::GuardianVerification(guardian_error) => {
                    error!(
                        federation_id = %federation.launch_id,
                        federation_name = %federation.name,
                        age_mins,
                        guardian_error = %guardian_error,
                        "Guardian verification failed, will retry on next poll cycle"
                    );
                }
                DeploymentError::KubernetesApply(k8s_error) => {
                    error!(
                        federation_id = %federation.launch_id,
                        federation_name = %federation.name,
                        age_mins,
                        k8s_error = %k8s_error,
                        "Kubernetes resource creation failed, will retry on next poll cycle"
                    );
                }
                DeploymentError::RenderFailed(render_error) => {
                    error!(
                        federation_id = %federation.launch_id,
                        federation_name = %federation.name,
                        age_mins,
                        render_error = %render_error,
                        "Failed to render deployment objects, will retry on next poll cycle"
                    );
                }
                DeploymentError::Other(other_error) => {
                    error!(
                        federation_id = %federation.launch_id,
                        federation_name = %federation.name,
                        age_mins,
                        other_error = %other_error,
                        "Deployment failed, will retry on next poll cycle"
                    );
                }
            }

            // Log additional guidance for long-running failures
            if age_mins > 120 {
                warn!(
                    federation_id = %federation.launch_id,
                    federation_name = %federation.name,
                    age_hours = age_mins / 60,
                    "Federation has been failing for over 2 hours - manual intervention may be required"
                );
            }

            // Leave in InProgress status to retry on next poll interval
            // The daemon will pick it up again since we now process InProgress federations
            Err(anyhow::anyhow!("Federation deployment failed: {:?}", e))
        }
    }
}

async fn deploy_federation(
    args: &FederationLauncherDaemonArgs,
    federation: &FederationLaunchConfiguration,
    kube_client: &kube::Client,
) -> Result<(Vec<Url>, Vec<Url>), DeploymentError> {
    use naming::*;

    let mut guardian_endpoints = Vec::new();
    let mut admin_ui_endpoints = Vec::new();

    let federation_raw_name = &federation.name;
    let created_at = &federation.created_at;
    let namespace = generate_namespace_name(federation_raw_name, created_at);
    let federation_resource_name =
        generate_federation_resource_name(federation_raw_name, created_at);
    let timeout = Duration::from_secs(args.deployment_timeout_secs);

    info!(
        federation_name = %federation.name,
        federation_resource_name = %federation_resource_name,
        namespace = %namespace,
        num_guardians = %federation.num_guardians,
        "Starting federation deployment"
    );

    // Deploy each guardian
    for i in 0..federation.num_guardians {
        let guardian_name = generate_guardian_name(i);
        let deployment_name = generate_guardian_deployment_name(federation_raw_name, created_at, i);

        info!(
            federation_name = %federation.name,
            guardian_name = %guardian_name,
            deployment_name = %deployment_name,
            "Deploying guardian"
        );

        let params = GuardianDeploymentParameters {
            federation_name: federation_raw_name.clone().into(),
            created_at: *created_at,
            guardian_index: i,
            image_name: args.image_name.clone(),
            az: args.az.clone(),
            fedimint_external_domain: args.fedimint_external_domain.clone(),
            bitcoin_rpc_password: args.bitcoin_rpc_password.clone(),
            bitcoin_network: args.bitcoin_network,
            use_http: args.use_http,
            test_mode: args.test_mode,
        };

        let objects = render_guardian_deployment_objects(params).map_err(|e| {
            DeploymentError::RenderFailed(e.context("Failed to render guardian objects"))
        })?;

        guardian_endpoints.push(objects.full_wss_fedimint_api_endpoint.clone());

        apply_guardian_deployment_objects(kube_client.clone(), objects)
            .await
            .map_err(|e| {
                DeploymentError::KubernetesApply(e.context("Failed to apply guardian objects"))
            })?;

        info!(
            federation_name = %federation.name,
            guardian_name = %guardian_name,
            deployment_name = %deployment_name,
            "Waiting for guardian deployment to be ready"
        );

        wait_for_deployment_ready(kube_client, &namespace, &deployment_name, timeout).await?;

        debug!(
            federation_name = %federation.name,
            guardian_name = %guardian_name,
            "Guardian deployed and ready"
        );
    }

    // Deploy admin UI
    info!(
        federation_name = %federation.name,
        "Deploying admin UI"
    );

    let ui_deployment_name = generate_ui_deployment_name(federation_raw_name, created_at);

    let ui_args = RenderGuardianUiArgs {
        federation_name: federation_raw_name.clone().into(),
        created_at: *created_at,
        image_name: args.ui_image_name.clone(),
        az: args.az.clone(),
        ui_external_domain: args.ui_external_domain.clone(),
        use_http: args.use_http,
    };

    let ui_objects = render_guardian_ui_objects(ui_args)
        .map_err(|e| DeploymentError::RenderFailed(e.context("Failed to render UI objects")))?;

    // Extract the admin UI endpoint
    let base_admin_ui_endpoint = ui_objects.url.to_string();

    apply_guardian_ui_objects(kube_client.clone(), ui_objects)
        .await
        .map_err(|e| DeploymentError::KubernetesApply(e.context("Failed to apply UI objects")))?;

    info!(
        federation_name = %federation.name,
        deployment_name = %ui_deployment_name,
        "Waiting for admin UI deployment to be ready"
    );

    wait_for_deployment_ready(kube_client, &namespace, &ui_deployment_name, timeout).await?;

    // Create admin UI endpoints with each guardian's API URL as a parameter
    // The admin UI can connect to different guardian APIs via the url parameter
    for guardian_endpoint in &guardian_endpoints {
        // URL-encode the WebSocket endpoint to ensure it's properly formatted as a
        // query parameter
        let encoded_ws_endpoint = urlencoding::encode(guardian_endpoint.as_str());
        let ui_url_with_guardian = Url::from_str(&format!(
            "{base_admin_ui_endpoint}?url={encoded_ws_endpoint}"
        ))
        .map_err(|e| {
            DeploymentError::Other(anyhow::anyhow!("Failed to parse admin UI URL: {}", e))
        })?;
        admin_ui_endpoints.push(ui_url_with_guardian);
    }

    info!(
        federation_name = %federation.name,
        guardian_count = guardian_endpoints.len(),
        "Federation deployment completed with all pods ready"
    );

    // Verify guardians are accessible via their API endpoints
    info!(
        federation_name = %federation.name,
        "Verifying guardians are accessible via API"
    );

    verify_guardians_accessible(
        &guardian_endpoints,
        args.readiness_check_max_retries,
        args.readiness_check_interval_secs,
        args.readiness_check_timeout_secs,
    )
    .await?;

    info!(
        federation_name = %federation.name,
        "All guardians verified as accessible and ready for setup"
    );

    Ok((guardian_endpoints, admin_ui_endpoints))
}

/// Get pod status details for a deployment
async fn get_deployment_pod_status(
    kube_client: &kube::Client,
    namespace: &str,
    deployment_name: &str,
) -> String {
    let mut details = String::new();
    let pods: Api<Pod> = Api::namespaced(kube_client.clone(), namespace);
    let params = ListParams::default().labels(&format!("app={deployment_name}"));

    if let Ok(pod_list) = pods.list(&params).await {
        for pod in pod_list.items {
            let pod_name = pod.metadata.name.as_deref().unwrap_or("unknown");

            if let Some(status) = &pod.status {
                // Check container statuses for issues
                if let Some(container_statuses) = &status.container_statuses {
                    for container in container_statuses {
                        if !container.ready {
                            details.push_str(&format!(
                                "Pod {pod_name}, container {}: ",
                                container.name
                            ));

                            if let Some(state) = &container.state {
                                if let Some(waiting) = &state.waiting {
                                    details.push_str(&format!(
                                        "Waiting - {}",
                                        waiting.reason.as_deref().unwrap_or("unknown")
                                    ));
                                    if let Some(message) = &waiting.message {
                                        details.push_str(&format!(" ({message})"));
                                    }
                                } else if let Some(terminated) = &state.terminated {
                                    details.push_str(&format!(
                                        "Terminated - exit code {code}",
                                        code = terminated.exit_code
                                    ));
                                    if let Some(reason) = &terminated.reason {
                                        details.push_str(&format!(" ({reason})"));
                                    }
                                }
                            }

                            if container.restart_count > 0 {
                                details.push_str(&format!(
                                    " [restarts: {count}]",
                                    count = container.restart_count
                                ));
                            }
                            details.push_str("; ");
                        }
                    }
                }
            }
        }
    }

    details
}

/// Wait for a deployment to be ready with at least one available replica
async fn wait_for_deployment_ready(
    kube_client: &kube::Client,
    namespace: &str,
    deployment_name: &str,
    timeout: Duration,
) -> Result<(), DeploymentError> {
    let deployments: Api<Deployment> = Api::namespaced(kube_client.clone(), namespace);
    let start_time = tokio::time::Instant::now();
    let mut last_progress_log = tokio::time::Instant::now();
    const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(30);

    loop {
        let elapsed = start_time.elapsed();
        if elapsed > timeout {
            return Err(DeploymentError::DeploymentTimeout {
                namespace: namespace.to_string(),
                deployment_name: deployment_name.to_string(),
                timeout_secs: timeout.as_secs(),
            });
        }

        // Log progress every 30 seconds
        if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
            let remaining = timeout.saturating_sub(elapsed);
            info!(
                deployment = %deployment_name,
                namespace = %namespace,
                elapsed_secs = elapsed.as_secs(),
                remaining_secs = remaining.as_secs(),
                "Still waiting for deployment to be ready"
            );
            last_progress_log = tokio::time::Instant::now();
        }

        match deployments.get(deployment_name).await {
            Ok(deployment) => {
                if let Some(status) = &deployment.status {
                    let replicas = status.replicas.unwrap_or(0);
                    let ready_replicas = status.ready_replicas.unwrap_or(0);
                    let available_replicas = status.available_replicas.unwrap_or(0);

                    debug!(
                        deployment = %deployment_name,
                        namespace = %namespace,
                        replicas = %replicas,
                        ready_replicas = %ready_replicas,
                        available_replicas = %available_replicas,
                        elapsed_secs = elapsed.as_secs(),
                        "Checking deployment status"
                    );

                    if available_replicas > 0 && ready_replicas > 0 {
                        info!(
                            deployment = %deployment_name,
                            namespace = %namespace,
                            elapsed_secs = elapsed.as_secs(),
                            "Deployment is ready"
                        );
                        return Ok(());
                    }

                    // Check for common failure conditions
                    if let Some(conditions) = &status.conditions {
                        for condition in conditions {
                            if condition.type_ == "Progressing"
                                && condition.status == "False"
                                && condition.reason.as_deref() == Some("ProgressDeadlineExceeded")
                            {
                                let reason =
                                    condition.message.as_deref().unwrap_or("unknown reason");
                                return Err(DeploymentError::Other(anyhow::anyhow!(
                                    "Deployment {namespace}/{deployment_name} failed to progress: {reason}"
                                )));
                            }
                        }
                    }

                    // Check pod status for more details when deployment is not ready
                    if replicas > 0 && ready_replicas == 0 {
                        // Get pod details to understand why they're not ready
                        let pod_status =
                            get_deployment_pod_status(kube_client, namespace, deployment_name)
                                .await;
                        if !pod_status.is_empty() {
                            debug!(
                                deployment = %deployment_name,
                                namespace = %namespace,
                                pod_status = %pod_status,
                                "Pod status details"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                warn!(
                    deployment = %deployment_name,
                    namespace = %namespace,
                    error = ?e,
                    "Failed to get deployment status"
                );
            }
        }

        time::sleep(Duration::from_secs(5)).await;
    }
}

/// Handle OG assignment for federations in InfrastructureReady status
async fn handle_og_assignment(
    db: &FederationLauncherDB,
    og_registry_db: &OgRegistryDB,
    federation: FederationLaunchConfiguration,
) -> Result<Option<FederationLaunchConfiguration>> {
    let required_ogs = federation.num_ogs as i64;

    // Check if OGs are already assigned with correct count and are not placeholders
    let has_placeholders = federation
        .og_user_ids
        .iter()
        .any(|og_id| og_id.as_ref().starts_with("placeholder-"));

    if federation.og_user_ids.len() == required_ogs as usize && !has_placeholders {
        info!(
            federation_id = %federation.launch_id,
            og_count = federation.og_user_ids.len(),
            required_ogs,
            "Correct number of OGs already assigned (no placeholders), transitioning to ReadyForDKG"
        );

        // Transition to ReadyForDKG
        db.update_status(
            federation.launch_id.clone(),
            FederationLaunchStatus::ReadyForDkg,
        )
        .await
        .context("Failed to update status to ReadyForDKG")?;

        return Ok(Some(federation));
    } else if !federation.og_user_ids.is_empty() && !has_placeholders {
        warn!(
            federation_id = %federation.launch_id,
            current_og_count = federation.og_user_ids.len(),
            required_ogs,
            "Incorrect number of non-placeholder OGs assigned, will reassign"
        );
        // Continue to reassignment logic below
    } else if has_placeholders {
        info!(
            federation_id = %federation.launch_id,
            current_og_count = federation.og_user_ids.len(),
            required_ogs,
            "Placeholder OGs detected, will replace with actual OGs from registry"
        );
        // Continue to reassignment logic below
    }

    // Try to get OGs from the registry
    match og_registry_db.get_random_active_ogs(required_ogs).await {
        Ok(available_ogs) if available_ogs.len() == required_ogs as usize => {
            info!(
                federation_id = %federation.launch_id,
                required_ogs,
                assigned_ogs = ?available_ogs,
                "Successfully selected OGs from registry"
            );

            // Assign OGs and transition to ReadyForDKG in a single transaction
            db.set_ogs_and_transition_to_ready_for_dkg(
                federation.launch_id.clone(),
                available_ogs.clone(),
            )
            .await
            .context("Failed to assign OGs and transition to ReadyForDKG")?;

            info!(
                federation_id = %federation.launch_id,
                "OGs assigned successfully, federation ready for DKG"
            );

            Ok(Some(federation))
        }
        Ok(available_ogs) => {
            warn!(
                federation_id = %federation.launch_id,
                required = required_ogs,
                available = available_ogs.len(),
                "Not enough active OGs available, will retry on next poll cycle"
            );
            // Stay in InfrastructureReady status for retry
            Ok(None)
        }
        Err(e) => {
            error!(
                federation_id = %federation.launch_id,
                error = ?e,
                "Failed to query OG registry, will retry on next poll cycle"
            );
            // Stay in InfrastructureReady status for retry
            Ok(None)
        }
    }
}
