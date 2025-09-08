use std::time::Duration;

use anyhow::Result;
use fedimint_api_client::api::DynGlobalApi;
use fedimint_core::admin_client::SetupStatus;
use fedimint_core::module::ApiAuth;
use fedimint_core::util::SafeUrl;
use thiserror::Error;
use tracing::{debug, info, warn};
use url::Url;

/// Custom error type for guardian status operations
#[derive(Debug, Error)]
#[allow(dead_code)]
pub(crate) enum GuardianStatusError {
    #[error("Failed to extract host from URL: {url}")]
    InvalidUrl { url: String },

    #[error("DNS resolution failed for host {host}: {source}")]
    DnsResolutionFailed {
        host: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse WebSocket URL: {url}")]
    UrlParseFailed {
        url: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("WebSocket connection failed to {endpoint}: {details}")]
    WebSocketConnectionFailed {
        endpoint: String,
        details: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Guardian API RPC call failed at {endpoint}: {details}")]
    ApiRpcFailed {
        endpoint: String,
        details: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("HTTP sanity check failed for {endpoint}: {details}")]
    HttpSanityCheckFailed { endpoint: String, details: String },

    #[error("Guardian verification timeout after {elapsed_secs} seconds for guardian {index} at {endpoint}")]
    VerificationTimeout {
        index: usize,
        endpoint: String,
        elapsed_secs: u64,
    },

    #[error("Guardian verification failed after {max_retries} retries for guardian {index} at {endpoint}")]
    MaxRetriesExceeded {
        index: usize,
        endpoint: String,
        max_retries: u32,
        #[source]
        source: Box<GuardianStatusError>,
    },
}

/// Client for checking federation guardian status
pub(crate) struct FedimintStatusClient {}

impl FedimintStatusClient {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for FedimintStatusClient {
    fn default() -> Self {
        Self::new()
    }
}

impl FedimintStatusClient {
    /// Check the setup status of a guardian at the given endpoint
    /// The endpoint should be a WebSocket URL (ws:// or wss://)
    pub async fn get_setup_status(
        &self,
        endpoint: &Url,
    ) -> Result<SetupStatus, GuardianStatusError> {
        debug!("Checking setup status at: {endpoint}");

        // First, try to resolve the hostname to verify DNS is working
        let host = endpoint
            .host_str()
            .ok_or_else(|| GuardianStatusError::InvalidUrl {
                url: endpoint.to_string(),
            })?;

        debug!("Attempting to resolve host: {host}");

        // Try to resolve DNS explicitly for better debugging
        match tokio::net::lookup_host(format!("{host}:80")).await {
            Ok(addrs) => {
                let addresses: Vec<_> = addrs.collect();
                debug!(
                    host = %host,
                    resolved_addresses = ?addresses,
                    "DNS resolution successful"
                );
            }
            Err(e) => {
                warn!(
                    host = %host,
                    error = %e,
                    "DNS resolution failed or timed out"
                );
                // Note: We don't return error here as DNS resolution might fail
                // but WebSocket could still work (e.g., through proxy)
            }
        }

        // Log the full URL for debugging
        info!(
            guardian_endpoint = %endpoint,
            host = %host,
            scheme = %endpoint.scheme(),
            "Attempting to connect to guardian API"
        );

        // Convert URL to SafeUrl (expects WebSocket URL)
        let safe_url =
            SafeUrl::parse(endpoint.as_str()).map_err(|e| GuardianStatusError::UrlParseFailed {
                url: endpoint.to_string(),
                source: anyhow::anyhow!("SafeUrl parse error: {}", e),
            })?;

        // For testing, we don't have authentication set up yet
        // The guardian should be in AwaitingLocalParams state
        let api_secret = None;

        // Create the fedimint API client
        let api = DynGlobalApi::from_setup_endpoint(safe_url, &api_secret)
            .await
            .map_err(|e| GuardianStatusError::WebSocketConnectionFailed {
                endpoint: endpoint.to_string(),
                details: "Failed to establish WebSocket connection. Check if the guardian pod is running and the ingress is configured correctly.".to_string(),
                source: anyhow::anyhow!("API client creation failed: {}", e),
            })?;

        // Get setup status without auth (for initial setup phase)
        // Note: This will fail if the guardian requires auth, but for fresh deployments
        // in AwaitingLocalParams state, it should work
        let auth = ApiAuth(String::new());
        let status = api
            .setup_status(auth)
            .await
            .map_err(|e| GuardianStatusError::ApiRpcFailed {
                endpoint: endpoint.to_string(),
                details: "The WebSocket connection was established but the RPC call failed. This might mean the guardian is not ready yet or requires authentication.".to_string(),
                source: anyhow::anyhow!("RPC call failed: {}", e),
            })?;

        info!(
            guardian_endpoint = %endpoint,
            status = ?status,
            "Successfully retrieved guardian status"
        );

        Ok(status)
    }
}

/// Verify that all guardians are accessible and in the expected state
pub(crate) async fn verify_guardians_accessible(
    guardian_endpoints: &[Url],
    max_retries: u32,
    retry_interval_secs: u64,
    timeout_secs: u64,
) -> Result<(), GuardianStatusError> {
    let client = FedimintStatusClient::new();
    let start_time = tokio::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let retry_interval = Duration::from_secs(retry_interval_secs);

    info!(
        guardian_count = guardian_endpoints.len(),
        timeout_mins = timeout_secs / 60,
        max_retries,
        retry_interval_secs,
        "Starting guardian accessibility verification"
    );

    for (i, ws_endpoint) in guardian_endpoints.iter().enumerate() {
        let mut retry_count = 0;
        let mut last_progress_log = tokio::time::Instant::now();
        const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(30);

        info!(
            guardian_index = i,
            endpoint = %ws_endpoint,
            "Checking guardian accessibility"
        );

        // Perform HTTP sanity check first
        let http_endpoint = ws_endpoint
            .as_str()
            .replace("ws://", "http://")
            .replace("wss://", "https://");

        info!(
            guardian_index = i,
            http_endpoint = %http_endpoint,
            "Performing HTTP sanity check before WebSocket connection"
        );

        // Try basic HTTP connectivity
        match reqwest::get(&http_endpoint).await {
            Ok(response) => {
                info!(
                    guardian_index = i,
                    status = %response.status(),
                    "HTTP sanity check successful - service is reachable"
                );

                // Log headers for debugging
                let headers = response.headers();
                if headers.contains_key("server") {
                    debug!(
                        guardian_index = i,
                        server = ?headers.get("server"),
                        "Server header from HTTP response"
                    );
                }
            }
            Err(e) => {
                warn!(
                    guardian_index = i,
                    error = %e,
                    "HTTP sanity check failed - service may not be reachable. Will still attempt WebSocket."
                );
            }
        }

        // Also try the metrics endpoint if available
        let metrics_url = format!("{base}/metrics", base = http_endpoint.trim_end_matches('/'));
        match reqwest::get(&metrics_url).await {
            Ok(response) => {
                debug!(
                    guardian_index = i,
                    status = %response.status(),
                    "Metrics endpoint is responding"
                );
            }
            Err(e) => {
                debug!(
                    guardian_index = i,
                    error = %e,
                    "Metrics endpoint not available (this is normal for some configurations)"
                );
            }
        }

        // Add a small delay before first WebSocket attempt to let service fully
        // initialize
        if retry_count == 0 {
            info!(
                guardian_index = i,
                "Waiting 2 seconds for guardian to fully initialize before WebSocket connection"
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        loop {
            let elapsed = start_time.elapsed();
            if elapsed > timeout {
                let elapsed_secs = elapsed.as_secs();
                return Err(GuardianStatusError::VerificationTimeout {
                    index: i,
                    endpoint: ws_endpoint.to_string(),
                    elapsed_secs,
                });
            }

            // Log progress every 30 seconds
            if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
                let remaining = timeout.saturating_sub(elapsed);
                info!(
                    guardian_index = i,
                    endpoint = %ws_endpoint,
                    elapsed_secs = elapsed.as_secs(),
                    remaining_secs = remaining.as_secs(),
                    retry_count,
                    "Still waiting for guardian to respond"
                );
                last_progress_log = tokio::time::Instant::now();
            }

            match client.get_setup_status(ws_endpoint).await {
                Ok(status) => {
                    use fedimint_core::admin_client::SetupStatus::*;

                    info!(
                        guardian_index = i,
                        endpoint = %ws_endpoint,
                        status = ?status,
                        elapsed_secs = elapsed.as_secs(),
                        "Guardian responded with status"
                    );

                    // Check if the status is valid for a fresh deployment
                    match status {
                        AwaitingLocalParams => {
                            info!(
                                guardian_index = i,
                                "Guardian is ready for setup (AwaitingLocalParams)"
                            );
                            break;
                        }
                        SharingConnectionCodes => {
                            warn!(
                                guardian_index = i,
                                "Guardian is already sharing connection codes (setup in progress?)"
                            );
                            break;
                        }
                        ConsensusIsRunning => {
                            warn!(
                                guardian_index = i,
                                "Guardian already has consensus running (reused deployment?)"
                            );
                            break;
                        }
                    }
                }
                Err(e) => {
                    retry_count += 1;

                    if retry_count >= max_retries {
                        return Err(GuardianStatusError::MaxRetriesExceeded {
                            index: i,
                            endpoint: ws_endpoint.to_string(),
                            max_retries,
                            source: Box::new(e),
                        });
                    }

                    // Log based on the specific error type
                    match &e {
                        GuardianStatusError::WebSocketConnectionFailed { .. } => {
                            if retry_count % 10 == 0 {
                                warn!(
                                    guardian_index = i,
                                    endpoint = %ws_endpoint,
                                    retry_count,
                                    max_retries,
                                    elapsed_secs = elapsed.as_secs(),
                                    error = %e,
                                    "Guardian WebSocket connection still failing"
                                );
                            } else {
                                debug!(
                                    guardian_index = i,
                                    retry_count,
                                    max_retries,
                                    error = %e,
                                    "WebSocket connection failed, retrying"
                                );
                            }
                        }
                        GuardianStatusError::ApiRpcFailed { .. } => {
                            if retry_count % 10 == 0 {
                                warn!(
                                    guardian_index = i,
                                    endpoint = %ws_endpoint,
                                    retry_count,
                                    max_retries,
                                    elapsed_secs = elapsed.as_secs(),
                                    error = %e,
                                    "Guardian API RPC call still failing"
                                );
                            } else {
                                debug!(
                                    guardian_index = i,
                                    retry_count,
                                    max_retries,
                                    error = %e,
                                    "API RPC call failed, retrying"
                                );
                            }
                        }
                        GuardianStatusError::InvalidUrl { .. }
                        | GuardianStatusError::UrlParseFailed { .. } => {
                            // These are permanent errors, log and fail immediately
                            warn!(
                                guardian_index = i,
                                endpoint = %ws_endpoint,
                                error = %e,
                                "URL error (permanent failure)"
                            );
                            return Err(e);
                        }
                        _ => {
                            debug!(
                                guardian_index = i,
                                retry_count,
                                max_retries,
                                error = %e,
                                "Guardian not ready yet, retrying"
                            );
                        }
                    }

                    tokio::time::sleep(retry_interval).await;
                }
            }
        }
    }

    info!(
        guardian_count = guardian_endpoints.len(),
        total_elapsed_secs = start_time.elapsed().as_secs(),
        "All guardians verified as accessible and ready"
    );
    Ok(())
}
