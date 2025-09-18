use concerto_common::*;
use std::collections::HashMap;
use tracing::{info, warn};

/// Parse and execute text commands from owner DMs
pub struct CommandParser;

impl CommandParser {
    pub fn parse(input: &str) -> Command {
        let parts: Vec<&str> = input.trim().split_whitespace().collect();
        
        match parts.get(0).map(|s| s.to_lowercase()).as_deref() {
            Some("help") => Command::Help,
            Some("status") => Command::Status,
            Some("subscriptions") | Some("subs") => Command::ListSubscriptions,
            Some("slots") => Command::ListSlots,
            Some("federations") | Some("feds") => Command::ListFederations,
            
            Some("propose") => {
                if parts.len() < 2 {
                    return Command::Error("Usage: propose <name> [slots] [total_slots]".to_string());
                }
                Command::ProposeFederation {
                    name: parts[1].to_string(),
                    my_slots: parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1),
                    total_slots: parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(4),
                }
            }
            
            Some("apply") => {
                if parts.len() < 2 {
                    return Command::Error("Usage: apply <federation_id> [slots]".to_string());
                }
                Command::ApplyToFederation {
                    federation_id: parts[1].to_string(),
                    slots: parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1),
                }
            }
            
            Some("approve") => {
                if parts.len() < 3 {
                    return Command::Error("Usage: approve <federation_id> <guardian_npub>".to_string());
                }
                Command::ApproveGuardian {
                    federation_id: parts[1].to_string(),
                    guardian_npub: parts[2].to_string(),
                }
            }
            
            Some("reject") => {
                if parts.len() < 4 {
                    return Command::Error("Usage: reject <federation_id> <guardian_npub> <reason>".to_string());
                }
                let reason = parts[3..].join(" ");
                Command::RejectGuardian {
                    federation_id: parts[1].to_string(),
                    guardian_npub: parts[2].to_string(),
                    reason,
                }
            }
            
            Some("allocate") => {
                if parts.len() < 3 {
                    return Command::Error("Usage: allocate <federation_id> <slot_count>".to_string());
                }
                Command::AllocateSlots {
                    federation_id: parts[1].to_string(),
                    count: parts[2].parse().unwrap_or(1),
                }
            }
            
            Some("release") => {
                if parts.len() < 2 {
                    return Command::Error("Usage: release <slot_id>".to_string());
                }
                Command::ReleaseSlot {
                    slot_id: parts[1].to_string(),
                }
            }
            
            Some("info") => {
                if parts.len() < 2 {
                    return Command::Error("Usage: info <federation_id>".to_string());
                }
                Command::FederationInfo {
                    federation_id: parts[1].to_string(),
                }
            }
            
            _ => Command::Unknown(input.to_string()),
        }
    }
    
    pub fn format_help() -> String {
        r#"📚 Guardianito Commands:

General:
  help                - Show this help message
  status              - Show guardian status
  subscriptions/subs  - List active subscriptions
  slots               - Show available and allocated slots
  federations/feds    - List federations

Federation Management:
  propose <name> [my_slots] [total_slots]
    - Propose a new federation
    
  apply <federation_id> [slots]
    - Apply to join a federation
    
  approve <federation_id> <guardian_npub>
    - Approve a guardian application (initiator only)
    
  reject <federation_id> <guardian_npub> <reason>
    - Reject a guardian application (initiator only)
    
  info <federation_id>
    - Get detailed federation information

Slot Management:
  allocate <federation_id> <count>
    - Allocate slots to a federation
    
  release <slot_id>
    - Release a slot

Examples:
  propose "My Federation" 2 4
  apply fed_abc123 1
  approve fed_abc123 npub1guardian..."#.to_string()
    }
    
    pub fn format_status(
        subscriptions: &[Subscription],
        federations: &HashMap<String, FederationParticipation>,
        available_slots: u32,
        allocated_slots: u32,
    ) -> String {
        format!(
            r#"🤖 Guardian Status:
━━━━━━━━━━━━━━━━━━
📊 Subscriptions: {}
🏛️ Federations: {}
📦 Available Slots: {}
🔒 Allocated Slots: {}
━━━━━━━━━━━━━━━━━━"#,
            subscriptions.len(),
            federations.len(),
            available_slots,
            allocated_slots
        )
    }
    
    pub fn format_subscriptions(subscriptions: &[Subscription]) -> String {
        if subscriptions.is_empty() {
            return "No active subscriptions".to_string();
        }
        
        let mut output = "📊 Active Subscriptions:\n".to_string();
        for sub in subscriptions {
            let status = if sub.is_active() { "✅" } else { "❌" };
            let slots = match &sub.plan {
                SubscriptionPlan::SlotBased(info) => info.total_slots,
            };
            output.push_str(&format!(
                "{} {} - {} slots (expires: {})\n",
                status,
                sub.id,
                slots,
                sub.valid_until.format("%Y-%m-%d")
            ));
        }
        output
    }
    
    pub fn format_federations(federations: &[Federation]) -> String {
        if federations.is_empty() {
            return "No federations found".to_string();
        }
        
        let mut output = "🏛️ Federations:\n".to_string();
        for fed in federations {
            let status_icon = match &fed.status {
                FederationStatus::Active { .. } => "🟢",
                FederationStatus::Forming { .. } => "🟡",
                FederationStatus::Proposed { .. } => "🔵",
                FederationStatus::Inactive { .. } => "🔴",
                _ => "⚪",
            };
            output.push_str(&format!(
                "{} {} - {} ({:?})\n",
                status_icon,
                fed.id,
                fed.name,
                fed.status
            ));
        }
        output
    }
    
    pub fn format_slots(slots: &[FedimintSlot]) -> String {
        if slots.is_empty() {
            return "No allocated slots".to_string();
        }
        
        let mut output = "📦 Allocated Slots:\n".to_string();
        for slot in slots {
            let status_icon = match &slot.state {
                SlotState::Running { .. } => "🟢",
                SlotState::Allocated { .. } => "🔵",
                SlotState::Launching { .. } => "🟡",
                SlotState::Stopped { .. } => "🔴",
                SlotState::Error { .. } => "❌",
                SlotState::Available => "⚪",
            };
            output.push_str(&format!(
                "{} {} - {:?}\n",
                status_icon,
                slot.id,
                slot.state
            ));
        }
        output
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Help,
    Status,
    ListSubscriptions,
    ListSlots,
    ListFederations,
    ProposeFederation {
        name: String,
        my_slots: u32,
        total_slots: u32,
    },
    ApplyToFederation {
        federation_id: String,
        slots: u32,
    },
    ApproveGuardian {
        federation_id: String,
        guardian_npub: String,
    },
    RejectGuardian {
        federation_id: String,
        guardian_npub: String,
        reason: String,
    },
    AllocateSlots {
        federation_id: String,
        count: u32,
    },
    ReleaseSlot {
        slot_id: String,
    },
    FederationInfo {
        federation_id: String,
    },
    Error(String),
    Unknown(String),
}