use std::sync::Arc;

use hephos_core::security::{
    AuditOutcome, ChainedSecretProvider, EnvSecretProvider, FileSecretProvider,
    InMemoryAuditLogger, SecretString, Secrets, reset_audit_logger, set_audit_logger,
};
use hephos_core::{Ctx, Identity, Principal, RateLimitConfig};
use uuid::Uuid;

#[test]
fn secret_value_is_strictly_redacted_in_debug_and_display() {
    let raw = "super-secret-auth-key-999";
    let secret = SecretString::from(raw);

    assert_eq!(secret.expose_secret(), raw);
    assert_eq!(format!("{secret:?}"), "[REDACTED]");
    assert_eq!(format!("{secret}"), "[REDACTED]");

    // Verify raw secret string never appears in debug representation
    assert!(!format!("{secret:?}").contains(raw));
}

#[test]
fn secret_serialization_always_redacts() {
    let secret = SecretString::from("raw-secret-token");
    let serialized = serde_json::to_string(&secret).unwrap();
    assert_eq!(serialized, r#""[REDACTED]""#);

    // Deserialization still reads the plain token from incoming configuration/storage
    let deserialized: SecretString = serde_json::from_str(r#""my-token""#).unwrap();
    assert_eq!(deserialized.expose_secret(), "my-token");
}

#[test]
fn secrets_env_file_and_chained_providers() {
    let temp_dir = std::env::temp_dir().join(format!("hephos-secrets-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let db_pass_file = temp_dir.join("db_password");
    std::fs::write(&db_pass_file, "hunter2\n").unwrap();

    let file_provider = Arc::new(FileSecretProvider::new(&temp_dir));
    let env_provider = Arc::new(EnvSecretProvider);
    let chain = ChainedSecretProvider::new(vec![file_provider, env_provider]);
    let secrets = Secrets::new(chain);

    // Read file secret
    let secret = secrets.get("db_password").unwrap().expect("secret found");
    assert_eq!(secret.expose_secret(), "hunter2");

    // Path traversal is strictly rejected
    let traversal = secrets.get("../../etc/shadow");
    assert!(traversal.is_err());

    // Missing secret
    assert!(secrets.get("missing_key_999").unwrap().is_none());
    assert!(secrets.require("missing_key_999").is_err());

    std::fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn ctx_audit_logging_captures_actor_and_metadata() {
    let logger = Arc::new(InMemoryAuditLogger::new());
    set_audit_logger(logger.clone());

    let user_id = Uuid::new_v4();
    let principal = Principal {
        id: user_id,
        roles: vec!["admin".into()],
    };
    let identity = Identity::User(principal);

    let ctx = Ctx::detached(()).with_identity(identity);
    ctx.audit(
        "user.change_password",
        "user",
        Some("target-user-456"),
        AuditOutcome::Success,
        serde_json::json!({
            "ip": "10.0.0.1",
            "method": "reset_link"
        }),
    );

    let events = logger.events();
    assert_eq!(events.len(), 1);
    let ev = &events[0];

    assert_eq!(ev.request_id, ctx.request_id());
    assert_eq!(ev.action, "user.change_password");
    assert_eq!(ev.resource_type, "user");
    assert_eq!(ev.resource_id.as_deref(), Some("target-user-456"));
    assert_eq!(ev.outcome, AuditOutcome::Success);
    assert_eq!(ev.metadata["ip"], "10.0.0.1");

    if let Identity::User(p) = &ev.actor {
        assert_eq!(p.id, user_id);
        assert!(p.has_role("admin"));
    } else {
        panic!("expected user identity in audit record");
    }

    reset_audit_logger();
}

#[test]
fn rate_limit_config_presets() {
    let standard = RateLimitConfig::standard();
    assert_eq!(standard.rps, 50);
    assert_eq!(standard.burst, 100);

    let sensitive = RateLimitConfig::sensitive();
    assert_eq!(sensitive.rps, 5);
    assert_eq!(sensitive.burst, 10);
}
