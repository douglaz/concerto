use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// A subscription represents a package of fedimint slots
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub owner_npub: String,
    pub plan: SubscriptionPlan,
    pub payment_info: PaymentInfo,
    pub valid_until: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubscriptionPlan {
    SlotBased(SlotBasedSubscriptionInfo),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotBasedSubscriptionInfo {
    pub name: String,
    pub total_slots: u32,
    pub price_sats: u64,
    pub duration_days: u32,
    pub features: HashSet<Feature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub enum Feature {
    BasicSupport,
    PrioritySupport,
    CustomConfiguration,
    HighAvailability,
    BackupRestore,
    Monitoring,
    DedicatedResources,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentInfo {
    pub method: PaymentMethod,
    pub status: PaymentStatus,
    pub last_payment_date: Option<DateTime<Utc>>,
    pub next_payment_date: Option<DateTime<Utc>>,
    pub payment_proof: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaymentMethod {
    Lightning { invoice: String },
    Onchain { address: String, txid: Option<String> },
    Ecash { mint_url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PaymentStatus {
    Pending,
    Active,
    Expired,
    Cancelled,
}

/// Proof of subscription ownership for slot allocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionProof {
    pub subscription_id: Uuid,
    pub owner_signature: String,  // Nostr signature proving ownership
    pub payment_receipt: PaymentReceipt,
    pub valid_until: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentReceipt {
    pub amount_sats: u64,
    pub paid_at: DateTime<Utc>,
    pub proof: String,  // Lightning preimage, tx hash, etc.
}

impl Subscription {
    pub fn new(
        owner_npub: String,
        plan: SubscriptionPlan,
        payment_info: PaymentInfo,
        duration_days: u32,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            owner_npub,
            plan,
            payment_info,
            valid_until: now + chrono::Duration::days(duration_days as i64),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn is_active(&self) -> bool {
        self.valid_until > Utc::now() && self.payment_info.status == PaymentStatus::Active
    }

    pub fn available_slots(&self, used_slots: u32) -> u32 {
        match &self.plan {
            SubscriptionPlan::SlotBased(info) => {
                if used_slots >= info.total_slots {
                    0
                } else {
                    info.total_slots - used_slots
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_creation() {
        let plan = SubscriptionPlan::SlotBased(SlotBasedSubscriptionInfo {
            name: "Basic Plan".to_string(),
            total_slots: 4,
            price_sats: 100000,
            duration_days: 30,
            features: HashSet::from([Feature::BasicSupport, Feature::Monitoring]),
        });

        let payment_info = PaymentInfo {
            method: PaymentMethod::Lightning {
                invoice: "lnbc...".to_string(),
            },
            status: PaymentStatus::Active,
            last_payment_date: Some(Utc::now()),
            next_payment_date: None,
            payment_proof: Some("preimage".to_string()),
        };

        let sub = Subscription::new(
            "npub1test...".to_string(),
            plan,
            payment_info,
            30,
        );

        assert!(sub.is_active());
        assert_eq!(sub.available_slots(0), 4);
        assert_eq!(sub.available_slots(2), 2);
        assert_eq!(sub.available_slots(4), 0);
        assert_eq!(sub.available_slots(5), 0);
    }
}