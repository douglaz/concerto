use std::str::FromStr;

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use fedimint_core::bitcoin::Network;
use futures::{pin_mut, StreamExt};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Namespace, PersistentVolumeClaim, Secret, Service};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{DeleteParams, PostParams};
use kube::Api;
use kube_runtime::reflector::Lookup;
use kube_runtime::{watcher, WatchStreamExt};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tracing::debug;
use url::Url;

#[derive(Debug, Clone)]
pub struct FederationName(String);

impl FederationName {
    pub fn normalized(&self) -> String {
        // make it compatible with kubernetes names
        self.0.replace(" ", "-").to_lowercase()
    }

    /// Get the raw name without any modifications
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for FederationName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

// Naming helper functions for Kubernetes resources and URLs
pub mod naming {
    use chrono::{DateTime, Datelike, Utc};

    /// Generate the date prefix from a timestamp in YYYYMMDD format
    pub fn generate_date_prefix(created_at: &DateTime<Utc>) -> String {
        format!(
            "{:04}{:02}{:02}",
            created_at.year(),
            created_at.month(),
            created_at.day()
        )
    }

    /// Generate namespace name with date prefix (fm-YYYYMMDD-name)
    /// The 'fm' prefix ensures DNS-1035 compliance (must start with letter)
    pub fn generate_namespace_name(name: &str, created_at: &DateTime<Utc>) -> String {
        let date_prefix = generate_date_prefix(created_at);
        let normalized_name = name.replace(" ", "-").to_lowercase();
        format!("fm-{date_prefix}-{normalized_name}")
    }

    /// Generate the base federation name used in deployments (fm-YYYYMMDD-name)
    pub fn generate_federation_resource_name(name: &str, created_at: &DateTime<Utc>) -> String {
        // Same as namespace for consistency
        generate_namespace_name(name, created_at)
    }

    /// Generate deployment name for a guardian
    pub fn generate_guardian_deployment_name(
        name: &str,
        created_at: &DateTime<Utc>,
        guardian_index: u8,
    ) -> String {
        let base_name = generate_federation_resource_name(name, created_at);
        let guardian_name = generate_guardian_name(guardian_index);
        format!("{base_name}-{guardian_name}")
    }

    /// Generate guardian name using NATO phonetic alphabet (e.g., "alpha",
    /// "bravo", "charlie")
    pub fn generate_guardian_name(guardian_index: u8) -> String {
        // GUARDIAN_NAMES is defined at the module level
        super::GUARDIAN_NAMES
            .get(guardian_index as usize)
            .unwrap_or(&"guardian-unknown")
            .to_string()
    }

    /// Generate UI deployment name
    pub fn generate_ui_deployment_name(name: &str, created_at: &DateTime<Utc>) -> String {
        let base_name = generate_federation_resource_name(name, created_at);
        format!("{base_name}-ui")
    }

    /// Generate service name for guardian P2P
    pub fn generate_guardian_p2p_service_name(
        guardian_name: &str,
        federation_resource_name: &str,
    ) -> String {
        format!("fedimint-{guardian_name}-{federation_resource_name}-p2p")
    }

    /// Generate service name for guardian HTTP
    pub fn generate_guardian_http_service_name(
        name: &str,
        created_at: &DateTime<Utc>,
        guardian_index: u8,
    ) -> String {
        let base_name = generate_federation_resource_name(name, created_at);
        let guardian_name = generate_guardian_name(guardian_index);
        format!("fedimint-{guardian_name}-{base_name}-http")
    }

    /// Generate PVC name for guardian
    pub fn generate_guardian_pvc_name(
        name: &str,
        created_at: &DateTime<Utc>,
        guardian_index: u8,
    ) -> String {
        let base_name = generate_federation_resource_name(name, created_at);
        let guardian_name = generate_guardian_name(guardian_index);
        format!("{base_name}-{guardian_name}-pvc")
    }

    pub fn build_websocket_fedimint_api_host(
        guardian_name: &str,
        federation_resource_name: &str,
        external_domain: &str,
    ) -> String {
        format!("api.{guardian_name}.{federation_resource_name}.{external_domain}")
    }

    pub fn build_fedimint_p2p_host(guardian_name: &str, federation_resource_name: &str) -> String {
        let service_name =
            generate_guardian_p2p_service_name(guardian_name, federation_resource_name);
        format!("{service_name}.{federation_resource_name}.svc.cluster.local")
    }

    pub fn build_full_fedimint_api_endpoint(
        guardian_name: &str,
        federation_resource_name: &str,
        external_domain: &str,
        use_http: bool,
    ) -> String {
        let protocol = if use_http { "ws" } else { "wss" };
        format!(
            "{}://{}",
            protocol,
            build_websocket_fedimint_api_host(
                guardian_name,
                federation_resource_name,
                external_domain
            )
        )
    }

    pub fn build_full_fedimint_p2p_endpoint(
        guardian_name: &str,
        federation_resource_name: &str,
    ) -> String {
        format!(
            "fedimint://{}:8173",
            build_fedimint_p2p_host(guardian_name, federation_resource_name)
        )
    }

    fn generate_ui_id(federation_resource_name: &str, az: &str) -> String {
        // Generate a hash for the UI ID to ensure uniqueness
        let hash = {
            use fedimint_core::bitcoin::hex::DisplayHex;
            let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
            sha2::Digest::update(&mut hasher, federation_resource_name.as_bytes());
            sha2::Digest::update(&mut hasher, az.as_bytes());
            sha2::Digest::finalize(hasher).as_hex().to_string()[..8].to_owned()
        };
        let ui_id = format!("{federation_resource_name}-{hash}");
        ui_id
    }

    pub fn generate_ui_admin_host(
        federation_resource_name: &str,
        az: &str,
        external_domain: &str,
    ) -> String {
        let ui_id = generate_ui_id(federation_resource_name, az);
        format!("{ui_id}.{external_domain}")
    }

    pub fn generate_ui_url(
        federation_resource_name: &str,
        az: &str,
        external_domain: &str,
        use_http: bool,
    ) -> String {
        let protocol = if use_http { "http" } else { "https" };
        format!(
            "{}://{}",
            protocol,
            generate_ui_admin_host(federation_resource_name, az, external_domain)
        )
    }
}

pub struct FederationDeploymentParameters {
    pub federation_name: FederationName,
    pub created_at: DateTime<Utc>,
    pub image_name: String,
    pub az: String,
    pub fedimint_external_domain: String,
    pub bitcoin_rpc_password: SecretString,
    pub bitcoin_network: Network,
    pub fedimints: u8,
}

pub struct GuardianDeploymentParameters {
    pub federation_name: FederationName,
    pub created_at: DateTime<Utc>,
    pub guardian_index: u8,
    pub image_name: String,
    pub az: String,
    pub fedimint_external_domain: String,
    pub bitcoin_rpc_password: SecretString,
    pub bitcoin_network: Network,
    pub use_http: bool,
    pub test_mode: bool,
}

pub async fn create_or_update<
    T: Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + serde::Serialize
        + kube::Resource
        + Send
        + 'static,
>(
    api: Api<T>,
    object: T,
) -> Result<()> {
    match api.create(&PostParams::default(), &object).await {
        Ok(_) => {
            debug!("{object:?}");
            watch_applied(api).await?;
            Ok(())
        }
        Err(kube::Error::Api(err)) if err.code == 409 => {
            // 409 Conflict means the object already exists, which is fine
            debug!("{object:?} already exists");
            replace(api, object).await?;
            Ok(())
        }
        Err(e) => bail!("Failed to create {object:?}: {e:?}"),
    }
}

async fn watch_applied<
    T: Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + serde::Serialize
        + kube::Resource
        + Send
        + 'static,
>(
    api: Api<T>,
) -> Result<()> {
    let stream = watcher(api, watcher::Config::default()).applied_objects();
    pin_mut!(stream);
    stream
        .next()
        .await
        .context("Failed to watch")?
        .context("Apply failed")?;
    Ok(())
}

async fn replace<
    T: Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + serde::Serialize
        + kube::Resource
        + Send
        + 'static,
>(
    api: Api<T>,
    object: T,
) -> Result<()> {
    let name = object
        .meta()
        .name
        .as_ref()
        .context("Resource missing 'name' field")?
        .to_owned();
    api.replace(&name, &PostParams::default(), &object)
        .await
        .with_context(|| format!("Failed to update {name}"))?;
    watch_applied(api).await?;
    debug!("Updated {object:?}");
    Ok(())
}

pub async fn create_if_not_exists<
    T: Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + serde::Serialize
        + kube::Resource
        + Send
        + 'static,
>(
    api: Api<T>,
    object: T,
) -> Result<()> {
    match api.create(&PostParams::default(), &object).await {
        Ok(_) => {
            debug!("Created {object:?}");
            watch_applied(api).await?;
            Ok(())
        }
        Err(kube::Error::Api(err)) if err.code == 409 => {
            // 409 Conflict means the object already exists, which is fine
            debug!("{object:?} already exists");
            Ok(())
        }
        Err(e) => bail!("Failed to create {object:?}: {e:?}"),
    }
}

pub async fn delete_if_exists<
    T: Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + serde::Serialize
        + kube::Resource
        + Send
        + 'static,
>(
    api: Api<T>,
    name: &str,
) -> Result<()> {
    match api.delete(name, &DeleteParams::default()).await {
        Ok(_) => {
            debug!("Deleted {name}");
            Ok(())
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            // 404 Not Found means the object doesn't exist, which is fine
            debug!("{name} doesn't exist");
            Ok(())
        }
        Err(e) => bail!("Failed to delete {name}: {e:?}"),
    }
}

pub struct RenderedGuardianObjects {
    pub namespace: Namespace,
    pub secret: Secret,
    pub deployment: Deployment,
    pub pvc: PersistentVolumeClaim,
    pub p2p_service: Service,
    pub http_service: Service,
    pub ingress: Ingress,
    pub full_wss_fedimint_api_endpoint: Url,
}

pub fn render_guardian_deployment_objects(
    GuardianDeploymentParameters {
        federation_name,
        created_at,
        guardian_index,
        image_name,
        az,
        fedimint_external_domain,
        bitcoin_rpc_password,
        bitcoin_network,
        use_http,
        test_mode,
    }: GuardianDeploymentParameters,
) -> anyhow::Result<RenderedGuardianObjects> {
    use naming::*;

    let federation_raw_name = federation_name.as_str();
    let namespace_name = generate_namespace_name(federation_raw_name, &created_at);

    // Check if we're in test mode to reduce resource requirements
    let (cpu_request, memory_request, cpu_limit, memory_limit) = if test_mode {
        // Minimal resources for tests
        ("10m", "128Mi", "500m", "512Mi")
    } else {
        // Production resources
        ("50m", "3Gi", "4", "3Gi")
    };

    let namespace_yaml = format!(
        r#"
apiVersion: v1
kind: Namespace
metadata:
  name: {namespace_name}
        "#,
    );
    let namespace: Namespace = serde_yaml::from_str(&namespace_yaml)?;

    let guardian_name = generate_guardian_name(guardian_index);
    let federation_resource_name =
        generate_federation_resource_name(federation_raw_name, &created_at);
    let deployment_name =
        generate_guardian_deployment_name(federation_raw_name, &created_at, guardian_index);
    let pvc_name = generate_guardian_pvc_name(federation_raw_name, &created_at, guardian_index);
    let ws_host = build_websocket_fedimint_api_host(
        &guardian_name,
        &federation_resource_name,
        &fedimint_external_domain,
    );
    let full_fedimint_api_endpoint = build_full_fedimint_api_endpoint(
        &guardian_name,
        &federation_resource_name,
        &fedimint_external_domain,
        use_http,
    );
    let full_fedimint_p2p_endpoint =
        build_full_fedimint_p2p_endpoint(&guardian_name, &federation_resource_name);

    let secret = format!(
        r#"
apiVersion: v1
kind: Secret
metadata:
  name: bitcoin-rpc-password
  namespace: {namespace_name}
type: Opaque
stringData:
  password: {bitcoin_rpc_password}
"#,
        bitcoin_rpc_password = bitcoin_rpc_password.expose_secret()
    );

    let deployment: String = format!(
        r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {deployment_name}
  namespace: {namespace_name}
spec:
  strategy:
    type: Recreate
  revisionHistoryLimit: 1
  replicas: 1
  selector:
    matchLabels:
      app: {deployment_name}
  template:
    metadata:
      labels:
        app: {deployment_name}
        app-service: {deployment_name}
        app.kubernetes.io/name: {deployment_name}
        app.kubernetes.io/instance: {deployment_name}
        app.kubernetes.io/component: {deployment_name}
        main-affinity-key: main-affinity-label
    spec:
      affinity:
        podAntiAffinity:
          requiredDuringSchedulingIgnoredDuringExecution:
            - labelSelector:
                matchExpressions:
                  - key: main-affinity-key
                    operator: In
                    values:
                      - main-affinity-label
              topologyKey: kubernetes.io/hostname
        nodeAffinity:
          requiredDuringSchedulingIgnoredDuringExecution:
            nodeSelectorTerms:
            - matchExpressions:
              - key: topology.kubernetes.io/zone
                operator: In
                values:
                - {az}
      containers:
        - name: fedimint
          image: {image_name}
          command:
            - sh
            - -c
            - |
              env FM_DEFAULT_BITCOIN_RPC_URL=http://bitcoin:${{BITCOIND_PASSWORD}}@bitcoind-rpc.bitcoind-service.svc.cluster.local:8332 fedimintd --api-url {full_fedimint_api_endpoint} --p2p-url {full_fedimint_p2p_endpoint}
          ports:
            - containerPort: 8173
              name: p2p
            - containerPort: 8080
              name: api
            - containerPort: 9999
              name: metrics
            - containerPort: 8175
              name: ui
          env:
            - name: FM_DEFAULT_BITCOIN_RPC_KIND
              value: bitcoind
            - name: BITCOIND_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: bitcoin-rpc-password
                  key: password
            - name: FM_BITCOIN_NETWORK
              value: {bitcoin_network}
            - name: FM_BIND_P2P
              value: 0.0.0.0:8173
            - name: FM_BIND_UI
              value: 0.0.0.0:8175
            - name: FM_BIND_API_WS
              value: 0.0.0.0:8080
            - name: FM_BIND_API_IROH
              value: 0.0.0.0:8081
            - name: FM_BIND_METRICS_API
              value: 0.0.0.0:9999
            - name: FM_DATA_DIR
              value: /var/lib/fedimint
            - name: FEDI_STABILITY_POOL_V2_MODULE_ENABLE
              value: "1"
            - name: RUST_LOG
              value: info,hyper=info,h2=info,rustls=info
          volumeMounts:
            - name: var-lib-fedimint
              mountPath: /var/lib/fedimint
          resources:
            requests:
              cpu: {cpu_request}
              memory: {memory_request}
            limits:
              cpu: {cpu_limit}
              memory: "{memory_limit}"
      volumes:
        - name: var-lib-fedimint
          persistentVolumeClaim:
            claimName: {pvc_name}
    "#
    );

    let pvc: String = format!(
        r#"
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: {pvc_name}
  namespace: {namespace_name}
spec:
  accessModes:
    - ReadWriteOnce
  storageClassName: {az}-ebs-sc
  resources:
    requests:
      storage: "10Gi"
  "#
    );

    let p2p_service_name =
        generate_guardian_p2p_service_name(&guardian_name, &federation_resource_name);
    let p2p_service = format!(
        r#"
apiVersion: v1
kind: Service
metadata:
  name: {p2p_service_name}
  namespace: {namespace_name}
spec:
  type: ClusterIP
  ports:
    - port: 8173
      name: p2p
      appProtocol: tcp
  selector:
    app: {deployment_name}
    "#
    );

    let http_service_name =
        generate_guardian_http_service_name(federation_raw_name, &created_at, guardian_index);
    let http_service = format!(
        r#"
apiVersion: v1
kind: Service
metadata:
  name: {http_service_name}
  namespace: {namespace_name}
spec:
  type: ClusterIP
  ports:
    - name: http
      port: 8080
    - name: metrics
      port: 9999
    - name: ui
      port: 8175
  selector:
    app: {deployment_name}
    "#
    );

    // Add SSL redirect annotation only if using HTTP (for testing)
    let ssl_redirect_annotation = if use_http {
        r#"nginx.ingress.kubernetes.io/ssl-redirect: "false""#
    } else {
        ""
    };

    let ingress = format!(
        r#"
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: fedimint-{guardian_name}-{federation_resource_name}-ingress
  namespace: {namespace_name}
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
    nginx.ingress.kubernetes.io/proxy-body-size: "0"
    nginx.ingress.kubernetes.io/proxy-read-timeout: "600"
    nginx.ingress.kubernetes.io/proxy-send-timeout: "600"
    {ssl_redirect_annotation}
spec:
  ingressClassName: nginx
  rules:
    - host: {ws_host}
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: {http_service_name}
                port:
                  name: http
    - host: metrics.{ws_host}
      http:
        paths:
          - path: /metrics
            pathType: Prefix
            backend:
              service:
                name: {http_service_name}
                port:
                  name: metrics
  tls:
    - secretName: fedimint-{guardian_name}-{federation_resource_name}-api-tls
      hosts:
        - {ws_host}
        - metrics.{ws_host}
    "#
    );

    Ok(RenderedGuardianObjects {
        namespace,
        secret: serde_yaml::from_str(&secret)?,
        deployment: serde_yaml::from_str(&deployment)?,
        pvc: serde_yaml::from_str(&pvc)?,
        p2p_service: serde_yaml::from_str(&p2p_service)?,
        http_service: serde_yaml::from_str(&http_service)?,
        ingress: serde_yaml::from_str(&ingress)?,
        full_wss_fedimint_api_endpoint: Url::from_str(&full_fedimint_api_endpoint)?,
    })
}

pub async fn apply_guardian_deployment_objects(
    kube_client: kube::Client,
    objects: RenderedGuardianObjects,
) -> anyhow::Result<()> {
    let namespace_name = objects
        .namespace
        .name()
        .context("namespace name is missing")?
        .to_string();
    create_or_update::<Namespace>(Api::all(kube_client.clone()), objects.namespace).await?;
    create_or_update::<Secret>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.secret,
    )
    .await?;
    create_or_update::<Deployment>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.deployment,
    )
    .await?;
    // TODO: implement an update method that is able to update the storage size
    // Perhaps we can create with a smaller size then just patch with the proper
    // size
    create_if_not_exists::<PersistentVolumeClaim>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.pvc,
    )
    .await?;
    create_or_update::<Service>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.p2p_service,
    )
    .await?;
    create_or_update::<Service>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.http_service,
    )
    .await?;
    create_or_update::<Ingress>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.ingress,
    )
    .await?;

    Ok(())
}

pub async fn delete_federation(
    kube_client: kube::Client,
    DeleteFederationArgs {
        federation_name,
        created_at,
    }: DeleteFederationArgs,
) -> Result<()> {
    use naming::*;
    let namespace = generate_namespace_name(federation_name.as_str(), &created_at);
    delete_if_exists::<Namespace>(Api::all(kube_client.clone()), &namespace).await?;
    Ok(())
}

pub struct RenderedGuardianUiObjects {
    pub namespace: Namespace,
    pub deployment: Deployment,
    pub service: Service,
    pub ingress: Ingress,
    pub url: Url,
}

pub fn render_guardian_ui_objects(
    RenderGuardianUiArgs {
        federation_name,
        created_at,
        image_name,
        az,
        ui_external_domain,
        use_http,
    }: RenderGuardianUiArgs,
) -> Result<RenderedGuardianUiObjects> {
    use naming::*;

    let federation_raw_name = federation_name.as_str();
    let namespace_name = generate_namespace_name(federation_raw_name, &created_at);
    let namespace_yaml = format!(
        r#"
apiVersion: v1
kind: Namespace
metadata:
  name: {namespace_name}
"#,
    );
    let namespace: Namespace = serde_yaml::from_str(&namespace_yaml)?;

    let federation_resource_name =
        generate_federation_resource_name(federation_raw_name, &created_at);
    let ui_deployment_name = generate_ui_deployment_name(federation_raw_name, &created_at);

    // Create deployment
    let deployment_yaml = format!(
        r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {ui_deployment_name}
  namespace: {namespace_name}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: {ui_deployment_name}
  template:
    metadata:
      labels:
        app: {ui_deployment_name}
        app-service: guardian-ui
    spec:
      containers:
        - name: guardian-ui
          image: {image_name}
          ports:
            - containerPort: 8080
              name: api
          env:
            - name: PORT
              value: "8080"
            - name: REACT_APP_TOS
              value: "BY GENERATING A PRIVATE KEY FOR THIS FEDIMINT, YOU AGREE TO SERVE AS A GUARDIAN FOR THE FEDERATION. IN YOUR ROLE AS A GUARDIAN, YOU ARE SOLELY RESPONSIBLE FOR: (1) ADMINISTERING THE PILOT FEDERATION; (2) ASSESSING APPLICABLE LAWS THAT CUSTODY AND TRANSFER OF BITCOIN MAY TRIGGER IN THE RELEVANT JURISDICTIONS; (3) ENSURING THAT THIRD PARTIES WILL USE THE FEDIMINT PROTOCOL IN COMPLIANCE WITH APPLICABLE LAWS AND WILL NOT UTILIZE THE FEDIMINT PROTOCOL IN WAYS THAT HARM OR DEFRAUD USERS; AND (4) PREVENTING USE OR ADDITION OF ANY CAPABILITIES ON THE FEDIMINT PROTOCOL IN EXCESS OF THOSE PROVIDED BY FEDI, INC. YOU ACKNOWLEDGE THAT FEDI, INC. IS NOT A PARTY TO THE PILOT FEDERATION, AND IS NOT RESPONSIBLE FOR ANY OF THE FOREGOING, AND CANNOT BE HELD LIABLE FOR ANY RESULTING LOSSES, DAMAGES, OBLIGATIONS, LIABILITIES, COSTS OR DEBT, AND EXPENSES (INCLUDING BUT NOT LIMITED TO ATTORNEYS' FEES)."
          resources:
            requests:
              cpu: "10m"
              memory: "100Mi"
            limits:
              cpu: "100m"
              memory: "100Mi"
"#
    );

    // Create service
    let service_yaml = format!(
        r#"
apiVersion: v1
kind: Service
metadata:
  name: {ui_deployment_name}
  namespace: {namespace_name}
spec:
  type: ClusterIP
  ports:
    - name: http
      port: 8080
  selector:
    app: {ui_deployment_name}
"#
    );

    // Create ingress
    let admin_host = generate_ui_admin_host(&federation_resource_name, &az, &ui_external_domain);

    // Add SSL redirect annotation only if using HTTP (for testing)
    let ssl_redirect_annotation = if use_http {
        r#"nginx.ingress.kubernetes.io/ssl-redirect: "false""#
    } else {
        ""
    };

    let ingress_yaml = format!(
        r#"
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: {ui_deployment_name}-ingress
  namespace: {namespace_name}
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
    nginx.ingress.kubernetes.io/proxy-body-size: "0"
    nginx.ingress.kubernetes.io/proxy-read-timeout: "600"
    nginx.ingress.kubernetes.io/proxy-send-timeout: "600"
    {ssl_redirect_annotation}
spec:
  ingressClassName: nginx
  tls:
    - secretName: {ui_deployment_name}-api-tls
      hosts:
        - {admin_host}
  rules:
    - host: {admin_host}
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: {ui_deployment_name}
                port:
                  number: 8080
"#
    );

    let object = RenderedGuardianUiObjects {
        namespace,
        deployment: serde_yaml::from_str(&deployment_yaml)?,
        service: serde_yaml::from_str(&service_yaml)?,
        ingress: serde_yaml::from_str(&ingress_yaml)?,
        url: std::str::FromStr::from_str(&generate_ui_url(
            &federation_resource_name,
            &az,
            &ui_external_domain,
            use_http,
        ))?,
    };

    Ok(object)
}

pub async fn apply_guardian_ui_objects(
    kube_client: kube::Client,
    objects: RenderedGuardianUiObjects,
) -> Result<()> {
    let namespace_name = objects
        .namespace
        .name()
        .context("namespace name is missing")?
        .to_string();
    create_or_update::<Namespace>(Api::all(kube_client.clone()), objects.namespace).await?;
    create_or_update::<Deployment>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.deployment,
    )
    .await?;
    create_or_update::<Service>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.service,
    )
    .await?;
    create_or_update::<Ingress>(
        Api::namespaced(kube_client.clone(), &namespace_name),
        objects.ingress,
    )
    .await?;
    Ok(())
}

pub fn print_rendered_guardian_deployment_objects(
    objects: RenderedGuardianObjects,
) -> anyhow::Result<()> {
    for s in [
        serde_yaml::to_string(&objects.namespace)?,
        serde_yaml::to_string(&objects.secret)?,
        serde_yaml::to_string(&objects.deployment)?,
        serde_yaml::to_string(&objects.pvc)?,
        serde_yaml::to_string(&objects.p2p_service)?,
        serde_yaml::to_string(&objects.http_service)?,
        serde_yaml::to_string(&objects.ingress)?,
    ] {
        println!("{s}");
        println!("---");
    }

    Ok(())
}

pub fn print_rendered_guardian_ui_objects(
    objects: RenderedGuardianUiObjects,
) -> anyhow::Result<()> {
    for s in [
        serde_yaml::to_string(&objects.namespace)?,
        serde_yaml::to_string(&objects.deployment)?,
        serde_yaml::to_string(&objects.service)?,
        serde_yaml::to_string(&objects.ingress)?,
    ] {
        println!("{s}");
        println!("---");
    }

    Ok(())
}

pub const GUARDIAN_NAMES: [&str; 15] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliett",
    "kilo", "lima", "mike", "november", "oscar",
];

pub async fn deploy_federation(
    kube_client: kube::Client,
    FederationDeploymentParameters {
        federation_name,
        created_at,
        image_name,
        az,
        fedimint_external_domain,
        bitcoin_rpc_password,
        bitcoin_network,
        fedimints,
    }: FederationDeploymentParameters,
) -> Result<()> {
    ensure!(fedimints > 0, "We need at least one guardian");
    ensure!(
        fedimints as usize <= GUARDIAN_NAMES.len(),
        "We support up to {max_guardians} guardians",
        max_guardians = GUARDIAN_NAMES.len()
    );
    for i in 0..fedimints {
        let parameters = GuardianDeploymentParameters {
            federation_name: federation_name.clone(),
            created_at,
            guardian_index: i,
            image_name: image_name.clone(),
            az: az.clone(),
            fedimint_external_domain: fedimint_external_domain.clone(),
            bitcoin_rpc_password: bitcoin_rpc_password.clone(),
            bitcoin_network,
            use_http: false,  // CLI mode uses HTTPS by default
            test_mode: false, // CLI mode uses production resources
        };
        let yamls = render_guardian_deployment_objects(parameters)?;
        apply_guardian_deployment_objects(kube_client.clone(), yamls).await?;
    }
    Ok(())
}

pub fn print_federation(
    FederationDeploymentParameters {
        federation_name,
        created_at,
        image_name,
        az,
        fedimint_external_domain,
        bitcoin_rpc_password,
        bitcoin_network,
        fedimints,
    }: FederationDeploymentParameters,
) -> Result<()> {
    for i in 0..fedimints {
        let parameters = GuardianDeploymentParameters {
            federation_name: federation_name.clone(),
            created_at,
            guardian_index: i,
            image_name: image_name.clone(),
            az: az.clone(),
            fedimint_external_domain: fedimint_external_domain.clone(),
            bitcoin_rpc_password: bitcoin_rpc_password.clone(),
            bitcoin_network,
            use_http: false,  // CLI mode uses HTTPS by default
            test_mode: false, // CLI mode uses production resources
        };
        let yamls = render_guardian_deployment_objects(parameters)?;
        print_rendered_guardian_deployment_objects(yamls)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Args)]
pub struct GuardianLauncherCmd {
    #[clap(subcommand)]
    command: GuardianLauncherSubCmd,
}

#[derive(Subcommand, Debug, Clone)]
pub enum GuardianLauncherSubCmd {
    /// Launch a Fedimint deployment on Kubernetes
    #[clap(about = "Launch a Fedimint deployment on Kubernetes")]
    LaunchFedimint(LaunchFedimintArgs),

    /// Render a Fedimint deployment on Kubernetes
    #[clap(about = "Render a Fedimint deployment on Kubernetes")]
    RenderFedimint(RenderFedimintArgs),

    /// Launch a federation of Fedimint deployments on Kubernetes
    #[clap(about = "Launch a federation of Fedimint deployments on Kubernetes")]
    LaunchFederation(LaunchFederationArgs),

    /// Render a federation of Fedimint deployments on Kubernetes
    #[clap(about = "Render a federation of Fedimint deployments on Kubernetes")]
    RenderFederation(RenderFederationArgs),
    /// Launch a guardian UI on Kubernetes
    #[clap(about = "Launch a guardian UI on Kubernetes")]
    LaunchGuardianUi(LaunchGuardianUiArgs),

    /// Render a guardian UI on Kubernetes
    #[clap(about = "Render a guardian UI on Kubernetes")]
    RenderGuardianUi(RenderGuardianUiCliArgs),

    /// Delete a Fedimint deployment on Kubernetes
    #[clap(about = "Delete a Federation deployment on Kubernetes")]
    DeleteFederation(DeleteFederationCliArgs),
}

#[derive(Debug, Clone, Args)]
pub struct LaunchFedimintArgs {
    #[clap(flatten)]
    args: RenderFedimintArgs,
}

#[derive(Debug, Clone, Args)]
pub struct RenderFedimintArgs {
    #[clap(long)]
    federation_name: FederationName,

    /// Guardian name (alpha, bravo, charlie, etc.)
    #[clap(long)]
    guardian_name: String,

    /// Fedimint image version to use
    #[clap(long)]
    image_name: String,

    /// Availability zone to use
    #[clap(long)]
    az: String,

    /// External domain to use
    #[clap(long)]
    fedimint_external_domain: String,

    /// Bitcoin RPC password
    #[clap(long)]
    bitcoin_rpc_password: SecretString,

    #[clap(long)]
    bitcoin_network: Network,

    /// Optional created_at timestamp (defaults to now)
    #[clap(long)]
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Args)]
pub struct LaunchGuardianUiArgs {
    #[clap(flatten)]
    args: RenderGuardianUiCliArgs,
}

#[derive(Debug, Clone, Args)]
pub struct RenderGuardianUiCliArgs {
    /// Federation name
    #[clap(long)]
    federation_name: FederationName,

    /// Guardian UI image version to use
    #[clap(long)]
    image_name: String,

    /// Availability zone to use
    #[clap(long)]
    az: String,

    /// External domain to use
    #[clap(long)]
    ui_external_domain: String,

    /// Optional created_at timestamp (defaults to now)
    #[clap(long)]
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteFederationCliArgs {
    /// Federation name
    #[clap(long)]
    federation_name: FederationName,

    /// Optional created_at timestamp (defaults to now)
    #[clap(long)]
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchGuardianUiResponse {
    pub url: Url,
}

#[derive(Debug, Clone, Args)]
pub struct LaunchFederationArgs {
    #[clap(flatten)]
    args: RenderFederationArgs,
}

#[derive(Debug, Clone, Args)]
pub struct RenderFederationArgs {
    /// Federation name (will also be used as the Kubernetes namespace)
    #[clap(long)]
    federation_name: FederationName,

    /// Fedimint image version to use
    #[clap(long)]
    image_name: String,

    /// Availability zone to use
    #[clap(long)]
    az: String,

    /// External domain to use
    #[clap(long)]
    fedimint_external_domain: String,

    /// Bitcoin RPC password
    #[clap(long)]
    bitcoin_rpc_password: SecretString,

    #[clap(long)]
    bitcoin_network: Network,

    /// Number of Fedimint deployments to launch
    #[clap(long)]
    fedimints: u8,

    /// Optional created_at timestamp (defaults to now)
    #[clap(long)]
    created_at: Option<DateTime<Utc>>,
}

// Note: These structs are no longer used in CLI mode, only programmatically
#[derive(Debug, Clone)]
pub struct RenderGuardianUiArgs {
    pub federation_name: FederationName,
    pub created_at: DateTime<Utc>,
    pub image_name: String,
    pub az: String,
    pub ui_external_domain: String,
    pub use_http: bool,
}

// Note: These structs are no longer used in CLI mode, only programmatically
#[derive(Debug, Clone)]
pub struct DeleteFederationArgs {
    pub federation_name: FederationName,
    pub created_at: DateTime<Utc>,
}

pub async fn run_guardian_launcher(guardian_cmd: GuardianLauncherCmd) -> Result<()> {
    let kube_client = kube::Client::try_default().await?;

    match guardian_cmd.command {
        GuardianLauncherSubCmd::LaunchFedimint(LaunchFedimintArgs { args }) => {
            let created_at = args.created_at.unwrap_or_else(Utc::now);
            // Convert guardian name to index
            let guardian_index = GUARDIAN_NAMES
                .iter()
                .position(|&name| name == args.guardian_name)
                .ok_or_else(|| {
                    let guardian_name = &args.guardian_name;
                    anyhow::anyhow!("Invalid guardian name: {guardian_name}. Must be one of: {GUARDIAN_NAMES:?}")
                })? as u8;
            let params = GuardianDeploymentParameters {
                federation_name: args.federation_name,
                created_at,
                guardian_index,
                image_name: args.image_name,
                az: args.az,
                fedimint_external_domain: args.fedimint_external_domain,
                bitcoin_rpc_password: args.bitcoin_rpc_password,
                bitcoin_network: args.bitcoin_network,
                use_http: false,  // CLI mode uses HTTPS by default
                test_mode: false, // CLI mode uses production resources
            };
            let objects = render_guardian_deployment_objects(params)?;
            apply_guardian_deployment_objects(kube_client, objects).await?;
        }
        GuardianLauncherSubCmd::RenderFedimint(args) => {
            let created_at = args.created_at.unwrap_or_else(Utc::now);
            // Convert guardian name to index
            let guardian_index = GUARDIAN_NAMES
                .iter()
                .position(|&name| name == args.guardian_name)
                .ok_or_else(|| {
                    let guardian_name = &args.guardian_name;
                    anyhow::anyhow!("Invalid guardian name: {guardian_name}. Must be one of: {GUARDIAN_NAMES:?}")
                })? as u8;
            let params = GuardianDeploymentParameters {
                federation_name: args.federation_name,
                created_at,
                guardian_index,
                image_name: args.image_name,
                az: args.az,
                fedimint_external_domain: args.fedimint_external_domain,
                bitcoin_rpc_password: args.bitcoin_rpc_password,
                bitcoin_network: args.bitcoin_network,
                use_http: false,  // CLI mode uses HTTPS by default
                test_mode: false, // CLI mode uses production resources
            };
            let objects = render_guardian_deployment_objects(params)?;
            print_rendered_guardian_deployment_objects(objects)?;
        }
        GuardianLauncherSubCmd::LaunchFederation(LaunchFederationArgs { args }) => {
            let created_at = args.created_at.unwrap_or_else(Utc::now);
            let params = FederationDeploymentParameters {
                federation_name: args.federation_name,
                created_at,
                image_name: args.image_name,
                az: args.az,
                fedimint_external_domain: args.fedimint_external_domain,
                bitcoin_rpc_password: args.bitcoin_rpc_password,
                bitcoin_network: args.bitcoin_network,
                fedimints: args.fedimints,
            };
            deploy_federation(kube_client, params).await?;
        }
        GuardianLauncherSubCmd::RenderFederation(args) => {
            let created_at = args.created_at.unwrap_or_else(Utc::now);
            let params = FederationDeploymentParameters {
                federation_name: args.federation_name,
                created_at,
                image_name: args.image_name,
                az: args.az,
                fedimint_external_domain: args.fedimint_external_domain,
                bitcoin_rpc_password: args.bitcoin_rpc_password,
                bitcoin_network: args.bitcoin_network,
                fedimints: args.fedimints,
            };
            print_federation(params)?;
        }
        GuardianLauncherSubCmd::LaunchGuardianUi(LaunchGuardianUiArgs { args }) => {
            let created_at = args.created_at.unwrap_or_else(Utc::now);
            let ui_args = RenderGuardianUiArgs {
                federation_name: args.federation_name,
                created_at,
                image_name: args.image_name,
                az: args.az,
                ui_external_domain: args.ui_external_domain,
                use_http: false, // CLI mode uses HTTPS by default
            };
            let objects = render_guardian_ui_objects(ui_args)?;
            let response = LaunchGuardianUiResponse {
                url: objects.url.clone(),
            };
            apply_guardian_ui_objects(kube_client, objects).await?;
            let json_output = serde_json::to_string_pretty(&response)?;
            println!("{json_output}");
        }
        GuardianLauncherSubCmd::RenderGuardianUi(args) => {
            let created_at = args.created_at.unwrap_or_else(Utc::now);
            let ui_args = RenderGuardianUiArgs {
                federation_name: args.federation_name,
                created_at,
                image_name: args.image_name,
                az: args.az,
                ui_external_domain: args.ui_external_domain,
                use_http: false, // CLI mode uses HTTPS by default
            };
            let objects = render_guardian_ui_objects(ui_args)?;
            print_rendered_guardian_ui_objects(objects)?;
        }
        GuardianLauncherSubCmd::DeleteFederation(args) => {
            let created_at = args.created_at.unwrap_or_else(Utc::now);
            let delete_args = DeleteFederationArgs {
                federation_name: args.federation_name,
                created_at,
            };
            delete_federation(kube_client, delete_args).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn test_naming_functions() -> anyhow::Result<()> {
        let chrono::MappedLocalTime::Single(created_at) =
            Utc.with_ymd_and_hms(2025, 8, 8, 12, 0, 0)
        else {
            bail!("Failed to create DateTime");
        };
        let federation_name = "test federation";

        // Test date prefix generation
        assert_eq!(naming::generate_date_prefix(&created_at), "20250808");

        // Test namespace name generation
        assert_eq!(
            naming::generate_namespace_name(federation_name, &created_at),
            "fm-20250808-test-federation"
        );

        // Test federation resource name
        assert_eq!(
            naming::generate_federation_resource_name(federation_name, &created_at),
            "fm-20250808-test-federation"
        );

        // Test guardian deployment name
        assert_eq!(
            naming::generate_guardian_deployment_name(federation_name, &created_at, 0),
            "fm-20250808-test-federation-alpha"
        );
        assert_eq!(
            naming::generate_guardian_deployment_name(federation_name, &created_at, 2),
            "fm-20250808-test-federation-charlie"
        );

        // Test guardian names using NATO phonetic alphabet
        assert_eq!(naming::generate_guardian_name(0), "alpha");
        assert_eq!(naming::generate_guardian_name(1), "bravo");
        assert_eq!(naming::generate_guardian_name(2), "charlie");
        assert_eq!(naming::generate_guardian_name(14), "oscar");

        // Test UI deployment name
        assert_eq!(
            naming::generate_ui_deployment_name(federation_name, &created_at),
            "fm-20250808-test-federation-ui"
        );

        // Test P2P service name
        assert_eq!(
            naming::generate_guardian_p2p_service_name("alpha", "fm-20250808-test-federation"),
            "fedimint-alpha-fm-20250808-test-federation-p2p"
        );

        // Test HTTP service name
        assert_eq!(
            naming::generate_guardian_http_service_name(federation_name, &created_at, 0),
            "fedimint-alpha-fm-20250808-test-federation-http"
        );

        // Test PVC name
        assert_eq!(
            naming::generate_guardian_pvc_name(federation_name, &created_at, 0),
            "fm-20250808-test-federation-alpha-pvc"
        );

        Ok(())
    }
}
