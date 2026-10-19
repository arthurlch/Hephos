//! WebSocket primitives, configuration, and connection hardening.
//!
//! Provides the blessed WebSocket surface:
//! - Size bounds ([`WsConfig::max_message_size`], [`WsConfig::max_frame_size`]) to prevent memory exhaustion.
//! - Heartbeat ping intervals and idle timeouts via [`WsHeartbeat`].
//! - Clean close frame semantics ([`close_normal`], [`close_away`], [`close_policy`], [`close_size`]).
//! - The canonical split-loop backpressure pattern.

use std::time::Duration;

pub use axum::extract::ws::{
    CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade, close_code,
};
pub use axum::response::Response;

/// Configuration for WebSocket connection limits, heartbeats, and timeouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsConfig {
    /// Maximum incoming message size in bytes (default: 64 KiB).
    pub max_message_size: usize,
    /// Maximum incoming frame size in bytes (default: 64 KiB).
    pub max_frame_size: usize,
    /// Interval between server-initiated ping frames (default: 15s).
    pub heartbeat_interval: Duration,
    /// Inactivity duration before terminating an unresponsive socket (default: 45s).
    pub idle_timeout: Duration,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            max_message_size: 64 * 1024,
            max_frame_size: 64 * 1024,
            heartbeat_interval: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(45),
        }
    }
}

impl WsConfig {
    /// Create a new configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum message size in bytes.
    pub fn max_message_size(mut self, size: usize) -> Self {
        self.max_message_size = size;
        self
    }

    /// Set maximum frame size in bytes.
    pub fn max_frame_size(mut self, size: usize) -> Self {
        self.max_frame_size = size;
        self
    }

    /// Set heartbeat ping interval.
    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Set idle timeout duration.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Apply message and frame size limits to an incoming [`WebSocketUpgrade`].
    pub fn apply(&self, upgrade: WebSocketUpgrade) -> WebSocketUpgrade {
        upgrade
            .max_message_size(self.max_message_size)
            .max_frame_size(self.max_frame_size)
    }
}

/// Helper to track heartbeat pings and idle timeouts in the connection loop.
pub struct WsHeartbeat {
    heartbeat_interval: tokio::time::Interval,
    idle_timeout: Duration,
    last_activity: tokio::time::Instant,
}

impl WsHeartbeat {
    /// Construct a heartbeat tracker with the specified interval and idle timeout.
    pub fn new(heartbeat_interval: Duration, idle_timeout: Duration) -> Self {
        let interval = tokio::time::interval_at(
            tokio::time::Instant::now() + heartbeat_interval,
            heartbeat_interval,
        );
        Self {
            heartbeat_interval: interval,
            idle_timeout,
            last_activity: tokio::time::Instant::now(),
        }
    }

    /// Construct a heartbeat tracker from a [`WsConfig`].
    pub fn from_config(config: &WsConfig) -> Self {
        Self::new(config.heartbeat_interval, config.idle_timeout)
    }

    /// Await the next heartbeat tick.
    pub async fn tick(&mut self) -> tokio::time::Instant {
        self.heartbeat_interval.tick().await
    }

    /// Record peer activity (message, pong, ping), resetting the idle timer.
    pub fn record_activity(&mut self) {
        self.last_activity = tokio::time::Instant::now();
    }

    /// Check whether the peer has exceeded the idle timeout.
    pub fn is_idle(&self) -> bool {
        self.last_activity.elapsed() >= self.idle_timeout
    }

    /// Remaining duration until idle timeout triggers.
    pub fn idle_remaining(&self) -> Duration {
        let elapsed = self.last_activity.elapsed();
        if elapsed >= self.idle_timeout {
            Duration::ZERO
        } else {
            self.idle_timeout - elapsed
        }
    }
}

/// Build a normal closure frame (code 1000).
pub fn close_normal(reason: impl Into<Utf8Bytes>) -> CloseFrame {
    CloseFrame {
        code: close_code::NORMAL,
        reason: reason.into(),
    }
}

/// Build a going-away / server shutdown closure frame (code 1001).
pub fn close_away(reason: impl Into<Utf8Bytes>) -> CloseFrame {
    CloseFrame {
        code: close_code::AWAY,
        reason: reason.into(),
    }
}

/// Build a policy violation closure frame (code 1008, e.g. for idle timeout).
pub fn close_policy(reason: impl Into<Utf8Bytes>) -> CloseFrame {
    CloseFrame {
        code: close_code::POLICY,
        reason: reason.into(),
    }
}

/// Build an oversized message closure frame (code 1009).
pub fn close_size(reason: impl Into<Utf8Bytes>) -> CloseFrame {
    CloseFrame {
        code: close_code::SIZE,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_builder() {
        let default_cfg = WsConfig::default();
        assert_eq!(default_cfg.max_message_size, 64 * 1024);
        assert_eq!(default_cfg.max_frame_size, 64 * 1024);
        assert_eq!(default_cfg.heartbeat_interval, Duration::from_secs(15));
        assert_eq!(default_cfg.idle_timeout, Duration::from_secs(45));

        let custom = WsConfig::new()
            .max_message_size(1024)
            .max_frame_size(512)
            .heartbeat_interval(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(10));

        assert_eq!(custom.max_message_size, 1024);
        assert_eq!(custom.max_frame_size, 512);
        assert_eq!(custom.heartbeat_interval, Duration::from_secs(5));
        assert_eq!(custom.idle_timeout, Duration::from_secs(10));
    }

    #[test]
    fn close_frame_constructors() {
        let normal = close_normal("all done");
        assert_eq!(normal.code, close_code::NORMAL);
        assert_eq!(normal.reason.as_str(), "all done");

        let away = close_away("server restarting");
        assert_eq!(away.code, close_code::AWAY);
        assert_eq!(away.reason.as_str(), "server restarting");

        let policy = close_policy("idle timeout");
        assert_eq!(policy.code, close_code::POLICY);
        assert_eq!(policy.reason.as_str(), "idle timeout");

        let size = close_size("message too large");
        assert_eq!(size.code, close_code::SIZE);
        assert_eq!(size.reason.as_str(), "message too large");
    }

    #[tokio::test]
    async fn heartbeat_and_idle_timeout_tracking() {
        let mut heartbeat = WsHeartbeat::new(Duration::from_millis(10), Duration::from_millis(50));
        assert!(!heartbeat.is_idle());

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!heartbeat.is_idle());

        heartbeat.record_activity();
        assert!(!heartbeat.is_idle());

        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(heartbeat.is_idle());
        assert_eq!(heartbeat.idle_remaining(), Duration::ZERO);

        heartbeat.record_activity();
        assert!(!heartbeat.is_idle());
        assert!(heartbeat.idle_remaining() > Duration::ZERO);
    }

    #[tokio::test]
    async fn upgrade_extractor_requires_valid_headers_and_on_upgrade() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        // 1. Missing OnUpgrade -> rejected
        let req_no_upgrade = Request::builder()
            .uri("/ws")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts, _) = req_no_upgrade.into_parts();
        assert!(
            WebSocketUpgrade::from_request_parts(&mut parts, &())
                .await
                .is_err()
        );

        // 2. With OnUpgrade -> succeeds
        let mut req_valid = Request::builder()
            .uri("/ws")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(axum::body::Body::empty())
            .unwrap();
        let on_upgrade = hyper::upgrade::on(&mut req_valid);
        let (mut parts, _) = req_valid.into_parts();
        parts.extensions.insert(on_upgrade);
        assert!(
            WebSocketUpgrade::from_request_parts(&mut parts, &())
                .await
                .is_ok()
        );
    }
}
