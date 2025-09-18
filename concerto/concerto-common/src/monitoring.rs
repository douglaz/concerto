use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Monitoring and observability system for Concerto
#[derive(Clone)]
pub struct MonitoringSystem {
    metrics: Arc<RwLock<MetricsCollector>>,
    alerts: Arc<RwLock<AlertManager>>,
    health_checker: Arc<RwLock<HealthChecker>>,
}

impl MonitoringSystem {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(MetricsCollector::new())),
            alerts: Arc::new(RwLock::new(AlertManager::new())),
            health_checker: Arc::new(RwLock::new(HealthChecker::new())),
        }
    }

    pub async fn record_metric(&self, metric: Metric) {
        let mut metrics = self.metrics.write().await;
        metrics.record(metric.clone());
        
        // Check for alerts
        let mut alerts = self.alerts.write().await;
        alerts.check_metric(&metric);
    }

    pub async fn get_metrics_summary(&self) -> MetricsSummary {
        let metrics = self.metrics.read().await;
        metrics.summarize()
    }

    pub async fn check_system_health(&self) -> SystemHealth {
        let mut checker = self.health_checker.write().await;
        checker.check_all_components().await
    }

    pub async fn get_active_alerts(&self) -> Vec<Alert> {
        let alerts = self.alerts.read().await;
        alerts.get_active()
    }
}

/// Metrics Collection
#[derive(Clone)]
pub struct MetricsCollector {
    counters: HashMap<String, u64>,
    gauges: HashMap<String, f64>,
    histograms: HashMap<String, Vec<f64>>,
    time_series: HashMap<String, VecDeque<TimeSeriesPoint>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Metric {
    Counter { name: String, value: u64 },
    Gauge { name: String, value: f64 },
    Histogram { name: String, value: f64 },
    TimeSeries { name: String, value: f64 },
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            gauges: HashMap::new(),
            histograms: HashMap::new(),
            time_series: HashMap::new(),
        }
    }

    pub fn record(&mut self, metric: Metric) {
        match metric {
            Metric::Counter { name, value } => {
                *self.counters.entry(name).or_insert(0) += value;
            }
            Metric::Gauge { name, value } => {
                self.gauges.insert(name, value);
            }
            Metric::Histogram { name, value } => {
                self.histograms.entry(name).or_insert_with(Vec::new).push(value);
            }
            Metric::TimeSeries { name, value } => {
                let series = self.time_series.entry(name).or_insert_with(VecDeque::new);
                series.push_back(TimeSeriesPoint {
                    timestamp: Utc::now(),
                    value,
                });
                // Keep only last 1000 points
                while series.len() > 1000 {
                    series.pop_front();
                }
            }
        }
    }

    pub fn summarize(&self) -> MetricsSummary {
        let mut summary = MetricsSummary::default();
        
        // Summarize counters
        for (name, value) in &self.counters {
            summary.counters.insert(name.clone(), *value);
        }
        
        // Summarize gauges
        for (name, value) in &self.gauges {
            summary.gauges.insert(name.clone(), *value);
        }
        
        // Summarize histograms
        for (name, values) in &self.histograms {
            if !values.is_empty() {
                let sum: f64 = values.iter().sum();
                let count = values.len() as f64;
                let mean = sum / count;
                let min = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
                let max = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
                
                summary.histograms.insert(name.clone(), HistogramSummary {
                    count: values.len(),
                    mean,
                    min,
                    max,
                    p50: Self::percentile(values, 0.5),
                    p95: Self::percentile(values, 0.95),
                    p99: Self::percentile(values, 0.99),
                });
            }
        }
        
        summary
    }

    fn percentile(values: &[f64], p: f64) -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let index = ((sorted.len() as f64) * p) as usize;
        sorted[index.min(sorted.len() - 1)]
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub counters: HashMap<String, u64>,
    pub gauges: HashMap<String, f64>,
    pub histograms: HashMap<String, HistogramSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramSummary {
    pub count: usize,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

/// Alert Management
#[derive(Clone)]
pub struct AlertManager {
    alerts: Vec<Alert>,
    rules: Vec<AlertRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: uuid::Uuid,
    pub severity: AlertSeverity,
    pub title: String,
    pub description: String,
    pub triggered_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub metric_name: String,
    pub metric_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub metric_name: String,
    pub condition: AlertCondition,
    pub severity: AlertSeverity,
    pub title: String,
    pub description_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertCondition {
    Above { threshold: f64 },
    Below { threshold: f64 },
    OutOfRange { min: f64, max: f64 },
    RateOfChange { threshold: f64 },
}

impl AlertManager {
    pub fn new() -> Self {
        Self {
            alerts: Vec::new(),
            rules: Self::default_rules(),
        }
    }

    fn default_rules() -> Vec<AlertRule> {
        vec![
            AlertRule {
                metric_name: "cpu_usage".to_string(),
                condition: AlertCondition::Above { threshold: 90.0 },
                severity: AlertSeverity::Warning,
                title: "High CPU Usage".to_string(),
                description_template: "CPU usage is at {value}%".to_string(),
            },
            AlertRule {
                metric_name: "memory_usage".to_string(),
                condition: AlertCondition::Above { threshold: 85.0 },
                severity: AlertSeverity::Warning,
                title: "High Memory Usage".to_string(),
                description_template: "Memory usage is at {value}%".to_string(),
            },
            AlertRule {
                metric_name: "error_rate".to_string(),
                condition: AlertCondition::Above { threshold: 0.05 },
                severity: AlertSeverity::Error,
                title: "High Error Rate".to_string(),
                description_template: "Error rate is {value}".to_string(),
            },
            AlertRule {
                metric_name: "federation_health".to_string(),
                condition: AlertCondition::Below { threshold: 0.8 },
                severity: AlertSeverity::Critical,
                title: "Federation Health Degraded".to_string(),
                description_template: "Federation health score is {value}".to_string(),
            },
        ]
    }

    pub fn check_metric(&mut self, metric: &Metric) {
        let (name, value) = match metric {
            Metric::Gauge { name, value } => (name.clone(), *value),
            Metric::TimeSeries { name, value } => (name.clone(), *value),
            _ => return,
        };

        let mut alerts_to_trigger = Vec::new();
        for rule in &self.rules {
            if rule.metric_name == name {
                let should_alert = match &rule.condition {
                    AlertCondition::Above { threshold } => value > *threshold,
                    AlertCondition::Below { threshold } => value < *threshold,
                    AlertCondition::OutOfRange { min, max } => value < *min || value > *max,
                    AlertCondition::RateOfChange { .. } => false, // TODO: Implement
                };

                if should_alert {
                    alerts_to_trigger.push((rule.clone(), value));
                }
            }
        }
        
        for (rule, value) in alerts_to_trigger {
            self.trigger_alert(&rule, value);
        }
    }

    fn trigger_alert(&mut self, rule: &AlertRule, value: f64) {
        let alert = Alert {
            id: uuid::Uuid::new_v4(),
            severity: rule.severity.clone(),
            title: rule.title.clone(),
            description: rule.description_template.replace("{value}", &format!("{:.2}", value)),
            triggered_at: Utc::now(),
            resolved_at: None,
            metric_name: rule.metric_name.clone(),
            metric_value: value,
        };

        self.alerts.push(alert);
    }

    pub fn get_active(&self) -> Vec<Alert> {
        self.alerts
            .iter()
            .filter(|a| a.resolved_at.is_none())
            .cloned()
            .collect()
    }
}

/// Health Checking
#[derive(Clone)]
pub struct HealthChecker {
    component_status: HashMap<String, ComponentHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    pub last_check: DateTime<Utc>,
    pub details: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemHealth {
    pub overall_status: HealthStatus,
    pub components: Vec<ComponentHealth>,
    pub uptime_seconds: u64,
    pub last_check: DateTime<Utc>,
}

impl HealthChecker {
    pub fn new() -> Self {
        Self {
            component_status: HashMap::new(),
        }
    }

    pub async fn check_all_components(&mut self) -> SystemHealth {
        // Check various components
        self.check_nostr_connectivity().await;
        self.check_database_health().await;
        self.check_federation_health().await;
        self.check_resource_usage().await;

        // Determine overall status
        let mut overall = HealthStatus::Healthy;
        let mut has_degraded = false;
        
        for component in self.component_status.values() {
            match &component.status {
                HealthStatus::Unhealthy { .. } => {
                    overall = component.status.clone();
                    break;
                }
                HealthStatus::Degraded { .. } => {
                    has_degraded = true;
                }
                _ => {}
            }
        }

        if has_degraded && matches!(overall, HealthStatus::Healthy) {
            overall = HealthStatus::Degraded {
                reason: "Some components are degraded".to_string(),
            };
        }

        SystemHealth {
            overall_status: overall,
            components: self.component_status.values().cloned().collect(),
            uptime_seconds: 0, // TODO: Track actual uptime
            last_check: Utc::now(),
        }
    }

    async fn check_nostr_connectivity(&mut self) {
        let health = ComponentHealth {
            name: "nostr_connectivity".to_string(),
            status: HealthStatus::Healthy, // TODO: Actual check
            last_check: Utc::now(),
            details: HashMap::new(),
        };
        self.component_status.insert("nostr_connectivity".to_string(), health);
    }

    async fn check_database_health(&mut self) {
        let health = ComponentHealth {
            name: "database".to_string(),
            status: HealthStatus::Healthy, // TODO: Actual check
            last_check: Utc::now(),
            details: HashMap::new(),
        };
        self.component_status.insert("database".to_string(), health);
    }

    async fn check_federation_health(&mut self) {
        let health = ComponentHealth {
            name: "federations".to_string(),
            status: HealthStatus::Healthy, // TODO: Actual check
            last_check: Utc::now(),
            details: HashMap::new(),
        };
        self.component_status.insert("federations".to_string(), health);
    }

    async fn check_resource_usage(&mut self) {
        let health = ComponentHealth {
            name: "resources".to_string(),
            status: HealthStatus::Healthy, // TODO: Actual check
            last_check: Utc::now(),
            details: HashMap::new(),
        };
        self.component_status.insert("resources".to_string(), health);
    }
}

/// Performance Tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub request_latency_ms: f64,
    pub throughput_rps: f64,
    pub error_rate: f64,
    pub active_connections: u32,
    pub queue_depth: u32,
}

/// Distributed Tracing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    pub trace_id: uuid::Uuid,
    pub span_id: uuid::Uuid,
    pub parent_span_id: Option<uuid::Uuid>,
    pub operation: String,
    pub start_time: DateTime<Utc>,
    pub duration_ms: u64,
    pub tags: HashMap<String, String>,
    pub events: Vec<SpanEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanEvent {
    pub timestamp: DateTime<Utc>,
    pub message: String,
    pub attributes: HashMap<String, String>,
}

/// Log Aggregation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub component: String,
    pub message: String,
    pub context: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Dashboard Data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub system_health: SystemHealth,
    pub active_alerts: Vec<Alert>,
    pub performance: PerformanceMetrics,
    pub federation_stats: FederationStats,
    pub economic_metrics: EconomicMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationStats {
    pub total_federations: u32,
    pub active_federations: u32,
    pub forming_federations: u32,
    pub total_guardians: u32,
    pub total_slots: u32,
    pub utilized_slots: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicMetrics {
    pub total_revenue_sats: u64,
    pub active_subscriptions: u32,
    pub average_slot_price: u64,
    pub utilization_rate: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_collection() -> anyhow::Result<()> {
        let monitoring = MonitoringSystem::new();
        
        // Record various metrics
        monitoring.record_metric(Metric::Counter {
            name: "requests".to_string(),
            value: 1,
        }).await;
        
        monitoring.record_metric(Metric::Gauge {
            name: "cpu_usage".to_string(),
            value: 45.5,
        }).await;
        
        monitoring.record_metric(Metric::Histogram {
            name: "latency".to_string(),
            value: 123.4,
        }).await;
        
        let summary = monitoring.get_metrics_summary().await;
        assert_eq!(summary.counters.get("requests"), Some(&1));
        assert_eq!(summary.gauges.get("cpu_usage"), Some(&45.5));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_alert_triggering() -> anyhow::Result<()> {
        let monitoring = MonitoringSystem::new();
        
        // Trigger high CPU alert
        monitoring.record_metric(Metric::Gauge {
            name: "cpu_usage".to_string(),
            value: 95.0,
        }).await;
        
        let alerts = monitoring.get_active_alerts().await;
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].title, "High CPU Usage");
        
        Ok(())
    }

    #[tokio::test]
    async fn test_health_checking() -> anyhow::Result<()> {
        let monitoring = MonitoringSystem::new();
        let health = monitoring.check_system_health().await;
        
        assert!(matches!(health.overall_status, HealthStatus::Healthy));
        assert!(!health.components.is_empty());
        
        Ok(())
    }
}