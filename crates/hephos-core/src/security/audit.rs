use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::Identity;

/// An immutable record of an actor performing an action on a target resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub timestamp_secs: i64,
    pub request_id: Uuid,
    pub actor: Identity,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub outcome: AuditOutcome,
    pub metadata: serde_json::Value,
}

impl AuditEvent {
    pub fn new(
        request_id: Uuid,
        actor: Identity,
        action: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: Option<impl Into<String>>,
        outcome: AuditOutcome,
        metadata: serde_json::Value,
    ) -> Self {
        let timestamp_secs = time::OffsetDateTime::now_utc().unix_timestamp();
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp_secs,
            request_id,
            actor,
            action: action.into(),
            resource_type: resource_type.into(),
            resource_id: resource_id.map(Into::into),
            outcome,
            metadata,
        }
    }
}

/// The outcome of an audited operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Success,
    Denied { reason: String },
    Failed { error: String },
}

/// Pluggable sink for audit records.
pub trait AuditLogger: Send + Sync + 'static {
    fn log(&self, event: &AuditEvent);
}

/// Default logger emitting structured events to `tracing` at INFO level.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracingAuditLogger;

impl AuditLogger for TracingAuditLogger {
    fn log(&self, event: &AuditEvent) {
        let (actor_type, actor_id) = match &event.actor {
            Identity::Anonymous => ("anonymous", None),
            Identity::User(p) => ("user", Some(p.id.to_string())),
        };
        let outcome_str = match &event.outcome {
            AuditOutcome::Success => "success",
            AuditOutcome::Denied { .. } => "denied",
            AuditOutcome::Failed { .. } => "failed",
        };

        tracing::info!(
            target: "hephos::audit",
            audit_id = %event.id,
            audit_timestamp_secs = event.timestamp_secs,
            audit_request_id = %event.request_id,
            audit_actor_type = actor_type,
            audit_actor_id = actor_id.as_deref().unwrap_or(""),
            audit_action = %event.action,
            audit_resource_type = %event.resource_type,
            audit_resource_id = event.resource_id.as_deref().unwrap_or(""),
            audit_outcome = outcome_str,
            metadata = %event.metadata,
            "audit event: {} on {} by {}",
            event.action,
            event.resource_type,
            actor_type
        );
    }
}

/// In-memory audit sink for tests and verification.
#[derive(Debug, Clone, Default)]
pub struct InMemoryAuditLogger {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl InMemoryAuditLogger {
    pub fn new() -> Self {
        InMemoryAuditLogger {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn events(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut lock) = self.events.lock() {
            lock.clear();
        }
    }
}

impl AuditLogger for InMemoryAuditLogger {
    fn log(&self, event: &AuditEvent) {
        if let Ok(mut lock) = self.events.lock() {
            lock.push(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_event_construction_and_in_memory_logger() {
        let logger = InMemoryAuditLogger::new();
        let event = AuditEvent::new(
            Uuid::new_v4(),
            Identity::Anonymous,
            "user.login",
            "user",
            Some("123"),
            AuditOutcome::Success,
            serde_json::json!({ "ip": "127.0.0.1" }),
        );
        logger.log(&event);

        let records = logger.events();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, "user.login");
        assert_eq!(records[0].resource_type, "user");
        assert_eq!(records[0].resource_id.as_deref(), Some("123"));
        assert_eq!(records[0].outcome, AuditOutcome::Success);
    }
}
