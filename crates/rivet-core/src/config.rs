use std::net::SocketAddr;
use std::time::Duration;

use crate::error::{Error, Result};

/// Process configuration, read once at startup from the environment.
///
/// Rivet reads only what the framework itself needs. Application config
/// (database URL, provider keys, feature flags) is read by the component that
/// owns it — e.g. `Db::connect_from_env` reads `DATABASE_URL`. There is no
/// central config struct an agent must thread everywhere.
///
/// | Variable             | Default        | Meaning                              |
/// | -------------------- | -------------- | ------------------------------------ |
/// | `RIVET_ADDR`         | `0.0.0.0:8080` | Listen address.                      |
/// | `RIVET_LOG`          | `pretty`       | `pretty` or `json` log output.       |
/// | `RIVET_TIMEOUT_SECS` | `30`           | Per-request response-generation timeout. |
/// | `RIVET_BODY_LIMIT`   | `2097152`      | Max request body size, in bytes (2 MiB). |
#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,
    pub log: LogFormat,
    /// Ceiling on how long a handler may take to produce a response. This bounds
    /// response *generation*, not body streaming, so SSE/WebSocket handlers (which
    /// return their response immediately) are unaffected.
    pub timeout: Duration,
    /// Maximum accepted request body size, in bytes. Oversized bodies get `413`.
    pub body_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::parse(
            std::env::var("RIVET_ADDR").ok().as_deref(),
            std::env::var("RIVET_LOG").ok().as_deref(),
            std::env::var("RIVET_TIMEOUT_SECS").ok().as_deref(),
            std::env::var("RIVET_BODY_LIMIT").ok().as_deref(),
        )
    }

    /// Pure parsing of the inputs, split from env reading so it is testable without
    /// mutating process environment (which is `unsafe` on edition 2024 and forbidden
    /// crate-wide).
    fn parse(
        addr: Option<&str>,
        log: Option<&str>,
        timeout_secs: Option<&str>,
        body_limit: Option<&str>,
    ) -> Result<Self> {
        let addr = match addr {
            Some(value) => value.parse().map_err(|_| {
                Error::invalid(format!("RIVET_ADDR is not a socket address: {value}"))
            })?,
            None => SocketAddr::from(([0, 0, 0, 0], 8080)),
        };

        let log = match log {
            Some("json") => LogFormat::Json,
            Some("pretty") | None => LogFormat::Pretty,
            Some(other) => {
                return Err(Error::invalid(format!(
                    "RIVET_LOG must be pretty|json: {other}"
                )));
            }
        };

        let timeout = match timeout_secs {
            Some(value) => {
                let secs = value.parse::<u64>().map_err(|_| {
                    Error::invalid(format!(
                        "RIVET_TIMEOUT_SECS must be a whole number: {value}"
                    ))
                })?;
                Duration::from_secs(secs)
            }
            None => Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        };

        let body_limit = match body_limit {
            Some(value) => value.parse::<usize>().map_err(|_| {
                Error::invalid(format!("RIVET_BODY_LIMIT must be a byte count: {value}"))
            })?,
            None => DEFAULT_BODY_LIMIT,
        };

        Ok(Config {
            addr,
            log,
            timeout,
            body_limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_unset() {
        let config = Config::parse(None, None, None, None).unwrap();
        assert_eq!(config.addr.port(), 8080);
        assert_eq!(config.log, LogFormat::Pretty);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.body_limit, 2 * 1024 * 1024);
    }

    #[test]
    fn parses_all_fields() {
        let config = Config::parse(
            Some("127.0.0.1:9000"),
            Some("json"),
            Some("5"),
            Some("1024"),
        )
        .unwrap();
        assert_eq!(config.addr.port(), 9000);
        assert_eq!(config.log, LogFormat::Json);
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.body_limit, 1024);
    }

    #[test]
    fn rejects_bad_addr() {
        let err = Config::parse(Some("not-an-addr"), None, None, None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_log_format() {
        let err = Config::parse(None, Some("verbose"), None, None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_timeout_and_body_limit() {
        assert!(matches!(
            Config::parse(None, None, Some("soon"), None).unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(None, None, None, Some("huge")).unwrap_err(),
            Error::Invalid(_)
        ));
    }
}
