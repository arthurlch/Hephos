use std::net::SocketAddr;
use std::time::Duration;

use crate::error::{Error, Result};

/// Process configuration, read once at startup from the environment.
///
/// Hephos reads only what the framework itself needs. Application config
/// (database URL, provider keys, feature flags) is read by the component that
/// owns it — e.g. `Db::connect_from_env` reads `DATABASE_URL`. There is no
/// central config struct an agent must thread everywhere.
///
/// | Variable             | Default        | Meaning                              |
/// | -------------------- | -------------- | ------------------------------------ |
/// | `HEPHOS_ADDR`         | `0.0.0.0:8080` | Listen address.                      |
/// | `HEPHOS_LOG`          | `pretty`       | `pretty` or `json` log output.       |
/// | `HEPHOS_TIMEOUT_SECS` | `30`           | Per-request response-generation timeout. |
/// | `HEPHOS_BODY_LIMIT`   | `2097152`      | Max request body size, in bytes (2 MiB). |
/// | `HEPHOS_HSTS`         | `off`          | Send `Strict-Transport-Security` (only behind TLS). |
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
    /// Send the `Strict-Transport-Security` header. Off by default: HSTS only makes
    /// sense when the app is served over TLS, and setting it over plain HTTP (or with
    /// a wrong max-age) can lock clients out of the domain.
    pub hsts: bool,
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
            std::env::var("HEPHOS_ADDR").ok().as_deref(),
            std::env::var("HEPHOS_LOG").ok().as_deref(),
            std::env::var("HEPHOS_TIMEOUT_SECS").ok().as_deref(),
            std::env::var("HEPHOS_BODY_LIMIT").ok().as_deref(),
            std::env::var("HEPHOS_HSTS").ok().as_deref(),
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
        hsts: Option<&str>,
    ) -> Result<Self> {
        let addr = match addr {
            Some(value) => value.parse().map_err(|_| {
                Error::invalid(format!("HEPHOS_ADDR is not a socket address: {value}"))
            })?,
            None => SocketAddr::from(([0, 0, 0, 0], 8080)),
        };

        let log = match log {
            Some("json") => LogFormat::Json,
            Some("pretty") | None => LogFormat::Pretty,
            Some(other) => {
                return Err(Error::invalid(format!(
                    "HEPHOS_LOG must be pretty|json: {other}"
                )));
            }
        };

        let timeout = match timeout_secs {
            Some(value) => {
                let secs = value.parse::<u64>().map_err(|_| {
                    Error::invalid(format!(
                        "HEPHOS_TIMEOUT_SECS must be a whole number: {value}"
                    ))
                })?;
                Duration::from_secs(secs)
            }
            None => Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        };

        let body_limit = match body_limit {
            Some(value) => value.parse::<usize>().map_err(|_| {
                Error::invalid(format!("HEPHOS_BODY_LIMIT must be a byte count: {value}"))
            })?,
            None => DEFAULT_BODY_LIMIT,
        };

        let hsts = matches!(
            hsts.map(str::to_ascii_lowercase).as_deref(),
            Some("1" | "on" | "true" | "yes")
        );

        Ok(Config {
            addr,
            log,
            timeout,
            body_limit,
            hsts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_unset() {
        let config = Config::parse(None, None, None, None, None).unwrap();
        assert_eq!(config.addr.port(), 8080);
        assert_eq!(config.log, LogFormat::Pretty);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.body_limit, 2 * 1024 * 1024);
        assert!(!config.hsts);
    }

    #[test]
    fn parses_all_fields() {
        let config = Config::parse(
            Some("127.0.0.1:9000"),
            Some("json"),
            Some("5"),
            Some("1024"),
            Some("on"),
        )
        .unwrap();
        assert_eq!(config.addr.port(), 9000);
        assert_eq!(config.log, LogFormat::Json);
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.body_limit, 1024);
        assert!(config.hsts);
    }

    #[test]
    fn hsts_is_lenient_boolean() {
        assert!(
            Config::parse(None, None, None, None, Some("TRUE"))
                .unwrap()
                .hsts
        );
        assert!(
            Config::parse(None, None, None, None, Some("1"))
                .unwrap()
                .hsts
        );
        assert!(
            !Config::parse(None, None, None, None, Some("off"))
                .unwrap()
                .hsts
        );
    }

    #[test]
    fn rejects_bad_addr() {
        let err = Config::parse(Some("not-an-addr"), None, None, None, None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_log_format() {
        let err = Config::parse(None, Some("verbose"), None, None, None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_timeout_and_body_limit() {
        assert!(matches!(
            Config::parse(None, None, Some("soon"), None, None).unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(None, None, None, Some("huge"), None).unwrap_err(),
            Error::Invalid(_)
        ));
    }
}
