//! Security primitives: secrets management, audit logging, and abuse protections.
//!
//! Every credential or API key is wrapped in a [`Secret`], preventing accidental
//! leaks in logs and debug formats. Security-sensitive actions emit an [`AuditEvent`]
//! for accountability and compliance.

pub mod audit;
pub mod secret;

pub use audit::{AuditEvent, AuditLogger, AuditOutcome, InMemoryAuditLogger, TracingAuditLogger};
pub use secret::{
    ChainedSecretProvider, EnvSecretProvider, FileSecretProvider, Secret, SecretProvider,
    SecretString, Secrets,
};

static CUSTOM_AUDIT_LOGGER: std::sync::RwLock<Option<std::sync::Arc<dyn AuditLogger>>> =
    std::sync::RwLock::new(None);

/// Register a global custom audit logger sink.
pub fn set_audit_logger(logger: std::sync::Arc<dyn AuditLogger>) {
    if let Ok(mut lock) = CUSTOM_AUDIT_LOGGER.write() {
        *lock = Some(logger);
    }
}

/// Reset the audit logger back to default tracing.
pub fn reset_audit_logger() {
    if let Ok(mut lock) = CUSTOM_AUDIT_LOGGER.write() {
        *lock = None;
    }
}

/// Record an audit event to the registered logger or standard tracing.
pub fn record_audit(event: &AuditEvent) {
    if let Ok(guard) = CUSTOM_AUDIT_LOGGER.read() {
        if let Some(logger) = guard.as_ref() {
            logger.log(event);
            return;
        }
    }
    TracingAuditLogger.log(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn custom_audit_logger_receives_global_events() {
        let in_memory = Arc::new(InMemoryAuditLogger::new());
        set_audit_logger(in_memory.clone());

        let event = AuditEvent::new(
            uuid::Uuid::new_v4(),
            crate::identity::Identity::Anonymous,
            "secret.access",
            "vault",
            Some("key_1"),
            AuditOutcome::Success,
            serde_json::json!({}),
        );
        record_audit(&event);

        let events = in_memory.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "secret.access");

        reset_audit_logger();
    }
}
