use concerto_common::*;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};
use ::url::Url;

/// Service Discovery System
/// Discovers and tracks services available in the Nostr ecosystem
pub struct ServiceDiscovery {
    discovered_services: HashMap<String, DiscoveredService>,
    service_filters: ServiceFilters,
    nostr_client: crate::nostr_client::NostrClient,
    last_scan: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredService {
    pub provider_npub: String,
    pub service_type: ServiceType,
    pub service_details: ServiceDetails,
    pub availability: ServiceAvailability,
    pub reputation_score: f32,
    pub discovered_at: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub metadata: ServiceMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDetails {
    pub name: String,
    pub description: String,
    pub api_endpoints: Vec<Url>,
    pub supported_features: Vec<String>,
    pub pricing: PricingInfo,
    pub terms_of_service: Option<Url>,
    pub regions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingInfo {
    pub currency: String,
    pub base_price: u64,
    pub pricing_model: PricingModel,
    pub discounts: Vec<Discount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discount {
    pub name: String,
    pub percentage: f32,
    pub conditions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceMetadata {
    pub uptime_percentage: f32,
    pub total_users: Option<u32>,
    pub active_federations: Option<u32>,
    pub reviews: Vec<ServiceReview>,
    pub certifications: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceReview {
    pub reviewer_npub: String,
    pub rating: u8, // 1-5
    pub comment: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceFilters {
    pub service_types: Option<Vec<ServiceType>>,
    pub min_reputation: Option<f32>,
    pub max_price: Option<u64>,
    pub required_features: Vec<String>,
    pub regions: Option<Vec<String>>,
    pub min_uptime: Option<f32>,
}

impl ServiceDiscovery {
    pub fn new(
        nostr_client: crate::nostr_client::NostrClient,
        filters: Option<ServiceFilters>,
    ) -> Self {
        Self {
            discovered_services: HashMap::new(),
            service_filters: filters.unwrap_or_default(),
            nostr_client,
            last_scan: None,
        }
    }

    /// Scan Nostr network for service advertisements
    pub async fn scan_services(&mut self) -> anyhow::Result<Vec<DiscoveredService>> {
        info!("Scanning Nostr network for services");
        
        // Query for service advertisement events
        let events = self.nostr_client.query_service_advertisements(None).await?;
        
        for event in events {
            if let Ok(service_ad) = ServiceAdvertisementEvent::from_nostr_event(&event) {
                self.process_service_advertisement(service_ad, event.pubkey).await?;
            }
        }

        // Query for FeLaaS provider advertisements specifically
        self.scan_felaas_providers().await?;
        
        // Query for Lightning gateway services
        self.scan_lightning_services().await?;
        
        // Query for stability pool providers
        self.scan_stability_pools().await?;
        
        self.last_scan = Some(chrono::Utc::now());
        
        Ok(self.get_filtered_services())
    }

    /// Find services matching specific criteria
    pub fn find_services(&self, criteria: ServiceCriteria) -> Vec<&DiscoveredService> {
        self.discovered_services
            .values()
            .filter(|service| self.matches_criteria(service, &criteria))
            .collect()
    }

    /// Get FeLaaS providers for federation hosting
    pub fn get_felaas_providers(&self) -> Vec<&DiscoveredService> {
        self.discovered_services
            .values()
            .filter(|s| matches!(s.service_type, ServiceType::FeLaaSProvider { .. }))
            .filter(|s| s.reputation_score >= 3.0)
            .collect()
    }

    /// Get Lightning gateway services
    pub fn get_lightning_gateways(&self) -> Vec<&DiscoveredService> {
        self.discovered_services
            .values()
            .filter(|s| matches!(s.service_type, ServiceType::LightningGateway { .. }))
            .collect()
    }

    /// Rate and rank services
    pub fn rank_services(&self, services: Vec<&DiscoveredService>) -> Vec<ServiceRanking> {
        let mut rankings: Vec<ServiceRanking> = services
            .into_iter()
            .map(|service| ServiceRanking {
                service: service.clone(),
                score: self.calculate_service_score(service),
            })
            .collect();

        rankings.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        rankings
    }

    /// Subscribe to real-time service updates
    pub async fn subscribe_to_updates(&self) -> anyhow::Result<()> {
        let filter = Filter::new()
            .kind(Kind::from(KIND_SERVICE_ADVERTISEMENT))
            .since(Timestamp::now());

        self.nostr_client.client.subscribe(vec![filter], None).await?;
        info!("Subscribed to service advertisement updates");
        
        Ok(())
    }

    /// Process incoming service advertisement
    async fn process_service_advertisement(
        &mut self,
        ad: ServiceAdvertisementEvent,
        pubkey: PublicKey,
    ) -> anyhow::Result<()> {
        let provider_npub = pubkey.to_bech32()?;
        
        // Extract service details from advertisement
        let service = DiscoveredService {
            provider_npub: provider_npub.clone(),
            service_type: ad.service_type,
            service_details: self.extract_service_details(&ad),
            availability: ad.terms.availability,
            reputation_score: self.calculate_initial_reputation(&ad),
            discovered_at: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            metadata: self.fetch_service_metadata(&provider_npub).await?,
        };

        // Apply filters
        if self.passes_filters(&service) {
            self.discovered_services.insert(provider_npub, service);
            info!("Discovered new service from {}", provider_npub);
        }

        Ok(())
    }

    async fn scan_felaas_providers(&mut self) -> anyhow::Result<()> {
        // Query specifically for FeLaaS providers
        let filter = Filter::new()
            .kind(Kind::from(KIND_SERVICE_ADVERTISEMENT))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::S),
                vec!["felaas_provider".to_string()],
            );

        let events = self.nostr_client.client.get_events_of(vec![filter], None).await?;
        
        for event in events {
            if let Ok(ad) = ServiceAdvertisementEvent::from_nostr_event(&event) {
                self.process_service_advertisement(ad, event.pubkey).await?;
            }
        }

        Ok(())
    }

    async fn scan_lightning_services(&mut self) -> anyhow::Result<()> {
        // Query for Lightning gateway services
        let filter = Filter::new()
            .kind(Kind::from(KIND_SERVICE_ADVERTISEMENT))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::S),
                vec!["lightning_gateway".to_string()],
            );

        let events = self.nostr_client.client.get_events_of(vec![filter], None).await?;
        
        for event in events {
            if let Ok(ad) = ServiceAdvertisementEvent::from_nostr_event(&event) {
                self.process_service_advertisement(ad, event.pubkey).await?;
            }
        }

        Ok(())
    }

    async fn scan_stability_pools(&mut self) -> anyhow::Result<()> {
        // Query for stability pool providers
        let filter = Filter::new()
            .kind(Kind::from(KIND_SERVICE_ADVERTISEMENT))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::S),
                vec!["stability_pool".to_string()],
            );

        let events = self.nostr_client.client.get_events_of(vec![filter], None).await?;
        
        for event in events {
            if let Ok(ad) = ServiceAdvertisementEvent::from_nostr_event(&event) {
                self.process_service_advertisement(ad, event.pubkey).await?;
            }
        }

        Ok(())
    }

    fn extract_service_details(&self, ad: &ServiceAdvertisementEvent) -> ServiceDetails {
        let (pricing, regions) = match &ad.service_type {
            ServiceType::FeLaaSProvider { regions, pricing } => {
                (pricing.clone(), regions.clone())
            }
            _ => (PricingModel::default(), vec![]),
        };

        ServiceDetails {
            name: format!("Service by {}", ad.provider_npub),
            description: String::new(),
            api_endpoints: vec![],
            supported_features: ad.requirements.clone(),
            pricing: PricingInfo {
                currency: "SATS".to_string(),
                base_price: pricing.base_price_sats,
                pricing_model: pricing,
                discounts: vec![],
            },
            terms_of_service: None,
            regions,
        }
    }

    fn calculate_initial_reputation(&self, _ad: &ServiceAdvertisementEvent) -> f32 {
        // Start with neutral reputation
        3.0
    }

    async fn fetch_service_metadata(&self, provider_npub: &str) -> anyhow::Result<ServiceMetadata> {
        // In real implementation, would fetch metadata from provider or reputation system
        Ok(ServiceMetadata {
            uptime_percentage: 99.0,
            total_users: None,
            active_federations: None,
            reviews: vec![],
            certifications: vec![],
        })
    }

    fn passes_filters(&self, service: &DiscoveredService) -> bool {
        // Check service type filter
        if let Some(ref types) = self.service_filters.service_types {
            if !types.iter().any(|t| std::mem::discriminant(t) == std::mem::discriminant(&service.service_type)) {
                return false;
            }
        }

        // Check reputation filter
        if let Some(min_rep) = self.service_filters.min_reputation {
            if service.reputation_score < min_rep {
                return false;
            }
        }

        // Check price filter
        if let Some(max_price) = self.service_filters.max_price {
            if service.service_details.pricing.base_price > max_price {
                return false;
            }
        }

        // Check uptime filter
        if let Some(min_uptime) = self.service_filters.min_uptime {
            if service.metadata.uptime_percentage < min_uptime {
                return false;
            }
        }

        true
    }

    fn matches_criteria(&self, service: &DiscoveredService, criteria: &ServiceCriteria) -> bool {
        // Match service type
        if let Some(ref service_type) = criteria.service_type {
            if std::mem::discriminant(&service.service_type) != std::mem::discriminant(service_type) {
                return false;
            }
        }

        // Match features
        for feature in &criteria.required_features {
            if !service.service_details.supported_features.contains(feature) {
                return false;
            }
        }

        // Match regions
        if let Some(ref region) = criteria.region {
            if !service.service_details.regions.contains(region) {
                return false;
            }
        }

        true
    }

    fn calculate_service_score(&self, service: &DiscoveredService) -> f32 {
        let mut score = 0.0;

        // Reputation weight: 40%
        score += service.reputation_score * 0.4;

        // Uptime weight: 30%
        score += (service.metadata.uptime_percentage / 100.0) * 5.0 * 0.3;

        // Price competitiveness weight: 20%
        // Lower price = higher score
        let price_score = if service.service_details.pricing.base_price > 0 {
            (1.0 / (service.service_details.pricing.base_price as f32).log10()) * 5.0
        } else {
            5.0
        };
        score += price_score * 0.2;

        // Features weight: 10%
        let feature_score = (service.service_details.supported_features.len() as f32 / 10.0).min(1.0) * 5.0;
        score += feature_score * 0.1;

        score
    }

    fn get_filtered_services(&self) -> Vec<DiscoveredService> {
        self.discovered_services
            .values()
            .filter(|s| self.passes_filters(s))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceCriteria {
    pub service_type: Option<ServiceType>,
    pub required_features: Vec<String>,
    pub region: Option<String>,
    pub max_price: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRanking {
    pub service: DiscoveredService,
    pub score: f32,
}

/// Monitor service health and availability
pub struct ServiceMonitor {
    services: Vec<String>, // NPUBs to monitor
    health_checks: HashMap<String, HealthCheckResult>,
    nostr_client: crate::nostr_client::NostrClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub service_npub: String,
    pub is_healthy: bool,
    pub response_time_ms: Option<u64>,
    pub last_check: chrono::DateTime<chrono::Utc>,
    pub error_message: Option<String>,
}

impl ServiceMonitor {
    pub fn new(services: Vec<String>, nostr_client: crate::nostr_client::NostrClient) -> Self {
        Self {
            services,
            health_checks: HashMap::new(),
            nostr_client,
        }
    }

    pub async fn check_all_services(&mut self) -> anyhow::Result<Vec<HealthCheckResult>> {
        let mut results = vec![];

        for service_npub in &self.services {
            let result = self.check_service_health(service_npub).await?;
            self.health_checks.insert(service_npub.clone(), result.clone());
            results.push(result);
        }

        Ok(results)
    }

    async fn check_service_health(&self, service_npub: &str) -> anyhow::Result<HealthCheckResult> {
        let start = std::time::Instant::now();
        
        // Send health check request via Nostr
        // In real implementation, would use specific health check protocol
        
        Ok(HealthCheckResult {
            service_npub: service_npub.to_string(),
            is_healthy: true, // Placeholder
            response_time_ms: Some(start.elapsed().as_millis() as u64),
            last_check: chrono::Utc::now(),
            error_message: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_filtering() -> anyhow::Result<()> {
        let filters = ServiceFilters {
            min_reputation: Some(3.5),
            max_price: Some(100000),
            min_uptime: Some(95.0),
            ..Default::default()
        };

        let service = DiscoveredService {
            provider_npub: "npub1test".to_string(),
            service_type: ServiceType::FeLaaSProvider {
                regions: vec!["US".to_string()],
                pricing: PricingModel::default(),
            },
            service_details: ServiceDetails {
                name: "Test Service".to_string(),
                description: String::new(),
                api_endpoints: vec![],
                supported_features: vec![],
                pricing: PricingInfo {
                    currency: "SATS".to_string(),
                    base_price: 50000,
                    pricing_model: PricingModel::default(),
                    discounts: vec![],
                },
                terms_of_service: None,
                regions: vec!["US".to_string()],
            },
            availability: ServiceAvailability::Always,
            reputation_score: 4.0,
            discovered_at: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            metadata: ServiceMetadata {
                uptime_percentage: 99.5,
                total_users: Some(100),
                active_federations: Some(10),
                reviews: vec![],
                certifications: vec![],
            },
        };

        // This service should pass all filters
        assert!(service.reputation_score >= 3.5);
        assert!(service.service_details.pricing.base_price <= 100000);
        assert!(service.metadata.uptime_percentage >= 95.0);
        
        Ok(())
    }
}