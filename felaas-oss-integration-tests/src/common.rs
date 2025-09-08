// Utilities for k3d-based integration testing

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use felaas_oss::{PgParams, PgPool, create_pg_pool};
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{ConfigMap, Event, Namespace, Pod, Service};
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding};
use k8s_openapi::serde::Deserialize;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use kube::{Client, Config};
use secrecy::{ExposeSecret, SecretString};
use tokio::{fs, time};
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct EnvConf {
    pub fedimint_external_domain: String,
    pub ui_external_domain: String,
    pub az: String,
    pub image_name: String,
    pub ui_image_name: String,
    pub bitcoin_rpc_password: SecretString,
}

/// Connect to the Kubernetes cluster using the kubeconfig from k3d
pub async fn connect_to_k8s(kubeconfig_path: Option<&str>) -> Result<Client> {
    let kubeconfig_path = match kubeconfig_path {
        Some(path) => PathBuf::from(path),
        None => {
            anyhow::bail!(
                "kubeconfig path is required. Please provide --kubeconfig parameter or set KUBECONFIG environment variable"
            )
        }
    };

    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig file not found at: {path}. Please run ./scripts/k3d-setup.sh first or verify the path",
            path = kubeconfig_path.display()
        );
    }

    info!(?kubeconfig_path, "Using kubeconfig");

    // Read kubeconfig content
    let kubeconfig_content = fs::read_to_string(&kubeconfig_path)
        .await
        .context("Failed to read kubeconfig")?;

    // Parse kubeconfig
    let kubeconfig = Config::from_custom_kubeconfig(
        serde_yaml::from_str(&kubeconfig_content)?,
        &Default::default(),
    )
    .await?;

    // Create client
    let client = Client::try_from(kubeconfig)?;

    // Verify connection by listing namespaces
    let namespaces: Api<Namespace> = Api::all(client.clone());
    namespaces
        .list(&Default::default())
        .await
        .context("Failed to connect to Kubernetes cluster")?;

    info!("Successfully connected to Kubernetes cluster");
    Ok(client)
}

/// Get PostgreSQL parameters for test environment with specific schema
pub fn get_test_pg_params(base_params: &PgParams, schema: &str) -> PgParams {
    let mut params = base_params.clone();
    params.pgschema = schema.to_string();
    // Override database to use the test database
    if params.pgdatabase.is_empty() {
        params.pgdatabase = "felaas_integration_tests".to_string();
    }
    params
}

/// Connect to PostgreSQL - uses NodePort exposed from k3d
pub async fn connect_to_postgres(params: &PgParams) -> Result<PgPool> {
    // PostgreSQL is exposed via NodePort 30432, mapped to host port 15432
    let pool = create_pg_pool(params)
        .await
        .context("Failed to create PostgreSQL connection pool")?;

    // Verify connection
    let client = pool.get().await?;
    client.execute("SELECT 1", &[]).await?;

    info!("Successfully connected to PostgreSQL on port 15432 (NodePort)");
    Ok(pool)
}

/// Create a test namespace and return its name
pub async fn create_test_namespace(client: &Client, prefix: &str, uuid: Uuid) -> Result<String> {
    let namespace_name = format!("{}-{}", prefix, uuid);
    let namespaces: Api<Namespace> = Api::all(client.clone());

    let namespace = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": namespace_name,
            "labels": {
                "test": "true",
                "test-uuid": uuid.to_string(),
            }
        }
    }))?;

    namespaces
        .create(&PostParams::default(), &namespace)
        .await?;
    info!(%namespace_name, "Created test namespace");
    Ok(namespace_name)
}

/// Clean up a test namespace
pub async fn cleanup_namespace(client: &Client, namespace_name: &str) -> Result<()> {
    let namespaces: Api<Namespace> = Api::all(client.clone());

    match namespaces
        .delete(namespace_name, &DeleteParams::default())
        .await
    {
        Ok(_) => {
            info!(%namespace_name, "Deleted test namespace");
            Ok(())
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            debug!(%namespace_name, "Namespace already deleted");
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Label nodes with availability zone
pub async fn label_nodes_with_zone(client: &Client, zone: &str) -> Result<()> {
    use k8s_openapi::api::core::v1::Node;
    use kube::api::Patch;
    use kube::api::PatchParams;

    let nodes: Api<Node> = Api::all(client.clone());
    let node_list = nodes.list(&Default::default()).await?;

    for node in node_list {
        if let Some(name) = node.metadata.name {
            if name.contains("agent") {
                let patch = serde_json::json!({
                    "metadata": {
                        "labels": {
                            "topology.kubernetes.io/zone": zone
                        }
                    }
                });

                nodes
                    .patch(&name, &PatchParams::default(), &Patch::Merge(patch))
                    .await?;
                debug!(%name, %zone, "Labeled node with availability zone");
            }
        }
    }

    info!(%zone, "Labeled all agent nodes with availability zone");
    Ok(())
}

/// Create storage classes needed for testing
pub async fn create_storage_classes(client: &Client) -> Result<()> {
    use k8s_openapi::api::storage::v1::StorageClass;

    let storage_classes: Api<StorageClass> = Api::all(client.clone());

    // Create test-az-ebs-sc if it doesn't exist
    match storage_classes.get("test-az-ebs-sc").await {
        Ok(_) => {
            info!("Storage class test-az-ebs-sc already exists");
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            let sc = serde_json::from_value(serde_json::json!({
                "apiVersion": "storage.k8s.io/v1",
                "kind": "StorageClass",
                "metadata": {
                    "name": "test-az-ebs-sc"
                },
                "provisioner": "rancher.io/local-path",
                "reclaimPolicy": "Delete",
                "volumeBindingMode": "WaitForFirstConsumer"
            }))?;

            storage_classes.create(&PostParams::default(), &sc).await?;
            info!("Created storage class test-az-ebs-sc");
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

/// Deploy PostgreSQL if not already running
pub async fn deploy_postgres_if_needed(client: &Client) -> Result<()> {
    let statefulsets: Api<StatefulSet> = Api::namespaced(client.clone(), "default");

    // Check if PostgreSQL is already deployed
    match statefulsets.get("postgres").await {
        Ok(_) => {
            info!("PostgreSQL StatefulSet already exists");
            return Ok(());
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            info!("PostgreSQL StatefulSet not found, will deploy");
        }
        Err(e) => return Err(e.into()),
    }

    // Deploy PostgreSQL
    let postgres_manifest = include_str!("../k8s_manifests/postgres.yaml");
    let postgres_resources: Vec<serde_yaml::Value> = serde_yaml::from_str(postgres_manifest)?;

    for resource in postgres_resources {
        let kind = resource
            .get("kind")
            .and_then(|k| k.as_str())
            .ok_or_else(|| anyhow::anyhow!("Resource missing kind"))?;

        match kind {
            "Service" => {
                let services: Api<Service> = Api::namespaced(client.clone(), "default");
                let service: Service = serde_yaml::from_value(resource)?;
                services.create(&PostParams::default(), &service).await?;
            }
            "StatefulSet" => {
                let statefulset: StatefulSet = serde_yaml::from_value(resource)?;
                statefulsets
                    .create(&PostParams::default(), &statefulset)
                    .await?;
            }
            _ => {
                warn!(%kind, "Unknown resource kind in PostgreSQL manifest");
            }
        }
    }

    info!("Deployed PostgreSQL StatefulSet");
    Ok(())
}

/// Wait for PostgreSQL to be ready
pub async fn wait_for_postgres(client: &Client) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");
    let timeout = Duration::from_secs(120);
    let start = std::time::Instant::now();

    loop {
        let pod_list = pods
            .list(&ListParams::default().labels("app=postgres"))
            .await?;

        if let Some(pod) = pod_list.items.first() {
            if let Some(status) = &pod.status {
                if let Some(conditions) = &status.conditions {
                    let ready = conditions
                        .iter()
                        .any(|c| c.type_ == "Ready" && c.status == "True");

                    if ready {
                        info!("PostgreSQL pod is ready");
                        return Ok(());
                    }
                }
            }
        }

        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for PostgreSQL to be ready");
        }

        time::sleep(Duration::from_secs(2)).await;
    }
}

/// Install NGINX Ingress Controller
pub async fn install_nginx_ingress_controller(client: &Client) -> Result<()> {
    use k8s_openapi::api::apps::v1::Deployment;

    // Check if already installed
    let namespaces: Api<Namespace> = Api::all(client.clone());
    match namespaces.get("ingress-nginx").await {
        Ok(_) => {
            // Check if the controller deployment exists
            let deployments: Api<Deployment> = Api::namespaced(client.clone(), "ingress-nginx");
            match deployments.get("ingress-nginx-controller").await {
                Ok(_) => {
                    info!("NGINX Ingress Controller already installed");
                    return Ok(());
                }
                Err(_) => {
                    info!("ingress-nginx namespace exists but controller not found, reinstalling");
                }
            }
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            info!("Installing NGINX Ingress Controller...");
        }
        Err(e) => return Err(e.into()),
    }

    // Apply the nginx ingress controller manifest
    // Using the same version as the reference project
    let manifest_url = "https://raw.githubusercontent.com/kubernetes/ingress-nginx/controller-v1.8.2/deploy/static/provider/cloud/deploy.yaml";

    // Use kubectl to apply the manifest
    info!("Applying NGINX Ingress Controller manifest");
    let output = Command::new("kubectl")
        .args(&["apply", "-f", manifest_url])
        .env(
            "KUBECONFIG",
            "/home/master/p/federation-tools-oss/k8s-config/kubeconfig.yaml",
        )
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to install NGINX Ingress Controller: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Wait for the controller to be ready
    info!("Waiting for NGINX Ingress Controller to be ready...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    info!("NGINX Ingress Controller installed successfully");
    Ok(())
}

/// Configure CoreDNS for internal domains
pub async fn configure_coredns(client: &Client, _kubeconfig_path: Option<&str>) -> Result<()> {
    let configmaps: Api<ConfigMap> = Api::namespaced(client.clone(), "kube-system");

    // Check if CoreDNS is already configured
    match configmaps.get("coredns-custom").await {
        Ok(_) => {
            info!("CoreDNS custom configuration already exists");
            return Ok(());
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            info!("CoreDNS custom configuration not found, will create");
        }
        Err(e) => return Err(e.into()),
    }

    // Create custom CoreDNS configuration
    let coredns_config = include_str!("../k8s_manifests/coredns-config.yaml");
    let configmap: ConfigMap = serde_yaml::from_str(coredns_config)?;
    configmaps
        .create(&PostParams::default(), &configmap)
        .await?;

    info!("Created CoreDNS custom configuration");
    Ok(())
}

/// Deploy daemon RBAC permissions
pub async fn deploy_daemon_rbac(client: &Client) -> Result<()> {
    // Check if RBAC already exists
    let cluster_roles: Api<ClusterRole> = Api::all(client.clone());
    match cluster_roles.get("felaas-daemon").await {
        Ok(_) => {
            info!("Daemon RBAC already exists");
            return Ok(());
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            info!("Daemon RBAC not found, will create");
        }
        Err(e) => return Err(e.into()),
    }

    // Create RBAC resources
    let rbac_manifest = include_str!("../k8s_manifests/daemon-rbac.yaml");
    let rbac_resources: Vec<serde_yaml::Value> = serde_yaml::from_str(rbac_manifest)?;

    for resource in rbac_resources {
        let kind = resource
            .get("kind")
            .and_then(|k| k.as_str())
            .ok_or_else(|| anyhow::anyhow!("Resource missing kind"))?;

        match kind {
            "ClusterRole" => {
                let role: ClusterRole = serde_yaml::from_value(resource)?;
                cluster_roles.create(&PostParams::default(), &role).await?;
            }
            "ClusterRoleBinding" => {
                let bindings: Api<ClusterRoleBinding> = Api::all(client.clone());
                let binding: ClusterRoleBinding = serde_yaml::from_value(resource)?;
                bindings.create(&PostParams::default(), &binding).await?;
            }
            _ => {
                warn!(%kind, "Unknown resource kind in RBAC manifest");
            }
        }
    }

    info!("Created daemon RBAC resources");
    Ok(())
}

/// Load Fedimint images into k3d
pub async fn load_fedimint_images(fedimint_image: &str, ui_image: &str) -> Result<()> {
    info!("Loading Fedimint images into k3d cluster");

    // Pull and load Fedimint guardian image
    info!("Pulling Fedimint guardian image: {}", fedimint_image);
    let output = Command::new("docker")
        .args(&["pull", fedimint_image])
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to pull Fedimint image: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    info!("Loading Fedimint guardian image into k3d");
    let output = Command::new("./bin/k3d")
        .args(&["image", "import", fedimint_image, "-c", "felaas-test"])
        .output()?;

    if !output.status.success() {
        warn!(
            "Failed to load Fedimint image into k3d (may already be loaded): {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Pull and load Fedimint UI image
    info!("Pulling Fedimint UI image: {}", ui_image);
    let output = Command::new("docker").args(&["pull", ui_image]).output()?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to pull Fedimint UI image: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    info!("Loading Fedimint UI image into k3d");
    let output = Command::new("./bin/k3d")
        .args(&["image", "import", ui_image, "-c", "felaas-test"])
        .output()?;

    if !output.status.success() {
        warn!(
            "Failed to load Fedimint UI image into k3d (may already be loaded): {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    info!("Fedimint images loaded successfully");
    Ok(())
}

/// Build and load daemon Docker image
pub async fn build_and_load_daemon_image(prebuilt_tag: Option<String>) -> Result<String> {
    if let Some(tag) = prebuilt_tag {
        info!(%tag, "Using pre-built daemon image");
        return Ok(tag);
    }

    // Build the Docker image
    info!("Building felaas-oss daemon Docker image");
    let output = Command::new("docker")
        .args(&["build", "-t", "felaas-oss:test", "-f", "Dockerfile", "."])
        .current_dir("/home/master/p/federation-tools-oss")
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to build Docker image: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Load image into k3d cluster
    info!("Loading image into k3d cluster");
    let output = Command::new("./bin/k3d")
        .args(&["image", "import", "felaas-oss:test", "-c", "felaas-test"])
        .current_dir("/home/master/p/federation-tools-oss")
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to load image into k3d: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    info!("Successfully built and loaded daemon image");
    Ok("felaas-oss:test".to_string())
}

/// Create ServiceAccount in namespace if it doesn't exist
async fn ensure_service_account(client: &Client, namespace: &str) -> Result<()> {
    use k8s_openapi::api::core::v1::ServiceAccount;

    let service_accounts: Api<ServiceAccount> = Api::namespaced(client.clone(), namespace);

    // Check if ServiceAccount already exists
    match service_accounts.get("felaas-daemon").await {
        Ok(_) => {
            debug!(%namespace, "ServiceAccount already exists");
            return Ok(());
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            debug!(%namespace, "ServiceAccount not found, will create");
        }
        Err(e) => return Err(e.into()),
    }

    // Create ServiceAccount
    let sa = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "ServiceAccount",
        "metadata": {
            "name": "felaas-daemon",
            "namespace": namespace,
        }
    }))?;

    service_accounts.create(&PostParams::default(), &sa).await?;
    info!(%namespace, "Created ServiceAccount felaas-daemon");
    Ok(())
}

/// Create ClusterRoleBinding for the namespace's ServiceAccount
async fn create_namespace_clusterrolebinding(client: &Client, namespace: &str) -> Result<()> {
    use k8s_openapi::api::rbac::v1::ClusterRoleBinding;

    let bindings: Api<ClusterRoleBinding> = Api::all(client.clone());
    let binding_name = format!("felaas-daemon-{}", namespace);

    // Check if already exists
    match bindings.get(&binding_name).await {
        Ok(_) => {
            debug!(%namespace, %binding_name, "ClusterRoleBinding already exists");
            return Ok(());
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            debug!(%namespace, %binding_name, "ClusterRoleBinding not found, will create");
        }
        Err(e) => return Err(e.into()),
    }

    // Create ClusterRoleBinding for this namespace's ServiceAccount
    let binding = serde_json::from_value(serde_json::json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRoleBinding",
        "metadata": {
            "name": binding_name,
        },
        "roleRef": {
            "apiGroup": "rbac.authorization.k8s.io",
            "kind": "ClusterRole",
            "name": "felaas-daemon",
        },
        "subjects": [{
            "kind": "ServiceAccount",
            "name": "felaas-daemon",
            "namespace": namespace,
        }]
    }))?;

    bindings.create(&PostParams::default(), &binding).await?;
    info!(%namespace, %binding_name, "Created ClusterRoleBinding for namespace");
    Ok(())
}

/// Deploy the daemon as a Kubernetes Job
pub async fn deploy_daemon_job(
    client: &Client,
    namespace: &str,
    image_tag: &str,
    pg_params: &PgParams,
    env_conf: &EnvConf,
) -> Result<String> {
    // Ensure ServiceAccount exists in the namespace
    ensure_service_account(client, namespace).await?;

    // Create ClusterRoleBinding for this namespace's ServiceAccount
    create_namespace_clusterrolebinding(client, namespace).await?;

    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let job_name = format!("felaas-daemon-{}", Uuid::new_v4());

    let job_manifest = serde_json::json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": &job_name,
            "namespace": namespace,
        },
        "spec": {
            "backoffLimit": 0,
            "template": {
                "spec": {
                    "serviceAccountName": "felaas-daemon",
                    "restartPolicy": "Never",
                    "containers": [{
                        "name": "daemon",
                        "image": image_tag,
                        "command": ["/usr/local/bin/felaas-oss"],
                        "args": [
                            "federation-launcher-daemon",
                            "--pghost", "postgres.default.svc.cluster.local",
                            "--pgport", "5432",
                            "--pguser", &pg_params.pguser,
                            "--pgpassword", pg_params.pgpassword.as_ref().map(|s| s.as_str()).unwrap_or(""),
                            "--pgdatabase", &pg_params.pgdatabase,
                            "--pgschema", &pg_params.pgschema,
                            "--fedimint-external-domain", &env_conf.fedimint_external_domain,
                            "--ui-external-domain", &env_conf.ui_external_domain,
                            "--az", &env_conf.az,
                            "--image-name", &env_conf.image_name,
                            "--ui-image-name", &env_conf.ui_image_name,
                            "--bitcoin-rpc-password", env_conf.bitcoin_rpc_password.expose_secret(),
                            "--bitcoin-network", "regtest",
                        ],
                        "env": [{
                            "name": "RUST_LOG",
                            "value": "info,felaas_oss=debug"
                        }]
                    }]
                }
            }
        }
    });

    let job: Job = serde_json::from_value(job_manifest)?;
    jobs.create(&PostParams::default(), &job).await?;

    info!(%job_name, %namespace, "Deployed daemon job");
    Ok(job_name)
}

/// Wait for daemon to be running
pub async fn wait_for_daemon_running(
    client: &Client,
    namespace: &str,
    job_name: &str,
    timeout: Duration,
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let start = std::time::Instant::now();

    loop {
        let pod_list = pods
            .list(&ListParams::default().labels(&format!("job-name={}", job_name)))
            .await?;

        if let Some(pod) = pod_list.items.first() {
            if let Some(status) = &pod.status {
                if let Some(phase) = &status.phase {
                    if phase == "Running" || phase == "Succeeded" {
                        info!(%job_name, %phase, "Daemon pod is ready");
                        return Ok(());
                    }
                    if phase == "Failed" {
                        anyhow::bail!("Daemon pod failed to start");
                    }
                }
            }
        }

        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for daemon to be ready");
        }

        time::sleep(Duration::from_secs(2)).await;
    }
}
