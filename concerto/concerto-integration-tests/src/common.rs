// Common utilities for Concerto integration testing
// Adapted from FeLaaS integration tests

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use deadpool_postgres::{Config as PgConfig, Runtime};
pub use deadpool_postgres::Pool as PgPool;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Pod, Service};
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding};
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use kube::{Client, Config};
use tokio::{fs, time};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Test environment configuration
pub struct EnvConf {
    pub nostr_relay_url: String,
    pub guardianito_image: String,
    pub felaas_image: String,
}

/// PostgreSQL connection parameters
#[derive(Clone)]
pub struct PgParams {
    pub pghost: String,
    pub pgport: u16,
    pub pguser: String,
    pub pgpassword: String,
    pub pgdatabase: String,
    pub pgschema: String,
}

/// Connect to the Kubernetes cluster using kubeconfig
pub async fn connect_to_k8s(kubeconfig_path: Option<&str>) -> Result<Client> {
    let kubeconfig_path = match kubeconfig_path {
        Some(path) => PathBuf::from(path),
        None => {
            // Try default k3d location
            let home = std::env::var("HOME").context("HOME not set")?;
            let k3d_config = PathBuf::from(format!("{}/.kube/k3d-concerto-test", home));
            if k3d_config.exists() {
                k3d_config
            } else {
                anyhow::bail!(
                    "No kubeconfig found. Please provide --kubeconfig or set KUBECONFIG env var"
                )
            }
        }
    };

    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig file not found at: {}. Please run k3d setup first",
            kubeconfig_path.display()
        );
    }

    info!("Using kubeconfig: {}", kubeconfig_path.display());

    // Read and parse kubeconfig
    let kubeconfig_content = fs::read_to_string(&kubeconfig_path)
        .await
        .context("Failed to read kubeconfig")?;

    let kubeconfig = Config::from_custom_kubeconfig(
        serde_yaml::from_str(&kubeconfig_content)?,
        &Default::default(),
    )
    .await?;

    let client = Client::try_from(kubeconfig)?;

    // Verify connection
    let namespaces: Api<Namespace> = Api::all(client.clone());
    namespaces
        .list(&Default::default())
        .await
        .context("Failed to connect to Kubernetes cluster")?;

    info!("Successfully connected to Kubernetes cluster");
    Ok(client)
}

/// Connect to PostgreSQL
pub async fn connect_to_postgres(params: &PgParams) -> Result<PgPool> {
    let mut config = PgConfig::new();
    config.host = Some(params.pghost.clone());
    config.port = Some(params.pgport);
    config.user = Some(params.pguser.clone());
    config.password = Some(params.pgpassword.clone());
    config.dbname = Some(params.pgdatabase.clone());

    let pool = config
        .create_pool(Some(Runtime::Tokio1), tokio_postgres::NoTls)
        .context("Failed to create PostgreSQL pool")?;

    // Verify connection
    let client = pool.get().await?;
    client.execute("SELECT 1", &[]).await?;

    info!(
        "Connected to PostgreSQL at {}:{}",
        params.pghost, params.pgport
    );
    Ok(pool)
}

/// Create a test namespace
pub async fn create_test_namespace(client: &Client, prefix: &str, uuid: Uuid) -> Result<String> {
    let namespace_name = format!("{}-{}", prefix, uuid.simple());

    let namespace = Namespace {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(namespace_name.clone()),
            labels: Some(
                [
                    ("test".to_string(), "true".to_string()),
                    ("test-type".to_string(), "integration".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        },
        ..Default::default()
    };

    let namespaces: Api<Namespace> = Api::all(client.clone());
    namespaces
        .create(&PostParams::default(), &namespace)
        .await?;

    info!("Created test namespace: {}", namespace_name);
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
            info!("Deleted test namespace: {}", namespace_name);
            Ok(())
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            // Already deleted
            Ok(())
        }
        Err(e) => {
            warn!("Failed to delete namespace {}: {:?}", namespace_name, e);
            Err(e.into())
        }
    }
}

/// Deploy PostgreSQL if not already present
pub async fn deploy_postgres_if_needed(client: &Client) -> Result<()> {
    let services: Api<Service> = Api::namespaced(client.clone(), "default");
    
    // Check if postgres already exists
    if services.get_opt("postgres").await?.is_some() {
        info!("PostgreSQL service already exists");
        return Ok(());
    }

    info!("Deploying PostgreSQL");
    
    // Deploy postgres statefulset and service
    // In real implementation, would apply k8s_manifests/postgres.yaml
    // For now, we'll assume it's deployed externally
    
    Ok(())
}

/// Deploy Nostr relay
pub async fn deploy_nostr_relay(client: &Client, image: &str) -> Result<()> {
    let services: Api<Service> = Api::namespaced(client.clone(), "default");
    
    // Check if relay already exists
    if services.get_opt("nostr-relay").await?.is_some() {
        info!("Nostr relay service already exists");
        return Ok(());
    }

    info!("Deploying Nostr relay with image: {}", image);
    
    // Create service
    let service = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {
            "name": "nostr-relay",
            "namespace": "default"
        },
        "spec": {
            "selector": {
                "app": "nostr-relay"
            },
            "ports": [{
                "port": 8008,
                "targetPort": 8080,
                "name": "websocket"
            }]
        }
    }))?;
    
    services.create(&PostParams::default(), &service).await?;
    
    // Create deployment
    let deployments: Api<Deployment> = Api::namespaced(client.clone(), "default");
    let deployment = serde_json::from_value(serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "nostr-relay",
            "namespace": "default"
        },
        "spec": {
            "replicas": 1,
            "selector": {
                "matchLabels": {
                    "app": "nostr-relay"
                }
            },
            "template": {
                "metadata": {
                    "labels": {
                        "app": "nostr-relay"
                    }
                },
                "spec": {
                    "containers": [{
                        "name": "relay",
                        "image": image,
                        "ports": [{
                            "containerPort": 8080,
                            "name": "websocket"
                        }],
                        "env": [{
                            "name": "RUST_LOG",
                            "value": "info,nostr_rs_relay=debug"
                        }]
                    }]
                }
            }
        }
    }))?;
    
    deployments.create(&PostParams::default(), &deployment).await?;
    
    info!("Nostr relay deployed");
    Ok(())
}

/// Wait for PostgreSQL to be ready
pub async fn wait_for_postgres(client: &Client) -> Result<()> {
    info!("Waiting for PostgreSQL to be ready");
    
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 60;
    
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
                        info!("PostgreSQL is ready");
                        return Ok(());
                    }
                }
            }
        }
        
        attempts += 1;
        if attempts >= MAX_ATTEMPTS {
            anyhow::bail!("Timeout waiting for PostgreSQL to be ready");
        }
        
        time::sleep(Duration::from_secs(2)).await;
    }
}

/// Wait for Nostr relay to be ready
pub async fn wait_for_nostr_relay(client: &Client) -> Result<()> {
    info!("Waiting for Nostr relay to be ready");
    
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 60;
    
    loop {
        let pod_list = pods
            .list(&ListParams::default().labels("app=nostr-relay"))
            .await?;
        
        if let Some(pod) = pod_list.items.first() {
            if let Some(status) = &pod.status {
                if let Some(conditions) = &status.conditions {
                    let ready = conditions
                        .iter()
                        .any(|c| c.type_ == "Ready" && c.status == "True");
                    
                    if ready {
                        info!("Nostr relay is ready");
                        return Ok(());
                    }
                }
            }
        }
        
        attempts += 1;
        if attempts >= MAX_ATTEMPTS {
            anyhow::bail!("Timeout waiting for Nostr relay to be ready");
        }
        
        time::sleep(Duration::from_secs(2)).await;
    }
}

/// Build and load Guardianito image into k3d
pub async fn build_and_load_guardianito_image() -> Result<String> {
    info!("Building Guardianito Docker image");
    
    let tag = format!("concerto-guardianito:test-{}", Uuid::new_v4().simple());
    
    // Build the image
    let output = Command::new("docker")
        .args(&[
            "build",
            "-f", "concerto-guardianito/Dockerfile", 
            "-t", &tag,
            ".",
        ])
        .current_dir("/home/master/p/federation-tools-oss/concerto")
        .output()
        .context("Failed to build Guardianito image")?;
    
    if !output.status.success() {
        anyhow::bail!(
            "Docker build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    
    // Load into k3d
    let output = Command::new("k3d")
        .args(&["image", "import", &tag, "-c", "concerto-test"])
        .output()
        .context("Failed to import image to k3d")?;
    
    if !output.status.success() {
        anyhow::bail!(
            "k3d image import failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    
    info!("Built and loaded image: {}", tag);
    Ok(tag)
}

/// Build and load FeLaaS image into k3d
pub async fn build_and_load_felaas_image() -> Result<String> {
    info!("Building FeLaaS Docker image");
    
    let tag = format!("concerto-felaas:test-{}", Uuid::new_v4().simple());
    
    // Build the image
    let output = Command::new("docker")
        .args(&[
            "build",
            "-f", "concerto-felaas/Dockerfile",
            "-t", &tag,
            ".",
        ])
        .current_dir("/home/master/p/federation-tools-oss/concerto")
        .output()
        .context("Failed to build FeLaaS image")?;
    
    if !output.status.success() {
        anyhow::bail!(
            "Docker build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    
    // Load into k3d
    let output = Command::new("k3d")
        .args(&["image", "import", &tag, "-c", "concerto-test"])
        .output()
        .context("Failed to import image to k3d")?;
    
    if !output.status.success() {
        anyhow::bail!(
            "k3d image import failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    
    info!("Built and loaded image: {}", tag);
    Ok(tag)
}

/// Deploy a Guardianito instance for testing
pub async fn deploy_guardianito_instance(
    client: &Client,
    namespace: &str,
    name: &str,
    image: &str,
    owner_npub: &str,
    guardian_nsec: &str,
    nostr_relays: &[String],
) -> Result<()> {
    info!("Deploying Guardianito instance: {}/{}", namespace, name);
    
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    
    let job = serde_json::from_value(serde_json::json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "template": {
                "spec": {
                    "restartPolicy": "Never",
                    "containers": [{
                        "name": "guardianito",
                        "image": image,
                        "command": ["/usr/local/bin/guardianito"],
                        "args": [
                            "--owner-npub", owner_npub,
                            "--guardian-nsec", guardian_nsec,
                            "--relays", nostr_relays.join(","),
                            "daemon"
                        ],
                        "env": [
                            {
                                "name": "RUST_LOG",
                                "value": "info,concerto_guardianito=debug"
                            },
                            {
                                "name": "DATABASE_URL",
                                "value": format!("postgres://postgres:postgres@postgres.default.svc.cluster.local:5432/{}", namespace)
                            }
                        ]
                    }]
                }
            }
        }
    }))?;
    
    jobs.create(&PostParams::default(), &job).await?;
    
    Ok(())
}

/// Initialize test database schema
pub async fn init_test_schema(pool: &PgPool, schema: &str) -> Result<()> {
    let client = pool.get().await?;
    
    // Create schema
    client
        .execute(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema), &[])
        .await?;
    
    info!("Created test schema: {}", schema);
    Ok(())
}

/// Clean up test schema
pub async fn cleanup_test_schema(pool: &PgPool, schema: &str) -> Result<()> {
    let client = pool.get().await?;
    
    client
        .execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", schema), &[])
        .await?;
    
    info!("Cleaned up test schema: {}", schema);
    Ok(())
}