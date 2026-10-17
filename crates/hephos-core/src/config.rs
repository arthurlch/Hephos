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
/// | `HEPHOS_CORS_ORIGINS` | *(unset)*      | Comma-separated allowed origins, or `*`. Unset = no CORS. |
// `#[non_exhaustive]`: the framework-config surface grows as new knobs are added, and a
// struct literal in app code must not break. Construct via `Config::from_env`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Config {
    /// Address the server binds and listens on (`HEPHOS_ADDR`).
    pub addr: SocketAddr,
    /// Log output format (`HEPHOS_LOG`): human-readable or structured JSON.
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
    /// Allowed CORS origins. `None` means no CORS layer (same-origin only, the safe
    /// default). `Some(["*"])` allows any origin; otherwise an explicit allow-list.
    pub cors_origins: Option<Vec<String>>,
}

/// How logs are formatted (`HEPHOS_LOG`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable, colorized output — for local development.
    Pretty,
    /// One JSON object per line — for log aggregation in production.
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
            std::env::var("HEPHOS_CORS_ORIGINS").ok().as_deref(),
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
        cors_origins: Option<&str>,
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

        let cors_origins = cors_origins.and_then(|value| {
            let origins: Vec<String> = value
                .split(',')
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .map(str::to_owned)
                .collect();
            (!origins.is_empty()).then_some(origins)
        });

        Ok(Config {
            addr,
            log,
            timeout,
            body_limit,
            hsts,
            cors_origins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(hsts: Option<&str>, cors: Option<&str>) -> Result<Config> {
        Config::parse(None, None, None, None, hsts, cors)
    }

    #[test]
    fn defaults_when_unset() {
        let config = Config::parse(None, None, None, None, None, None).unwrap();
        assert_eq!(config.addr.port(), 8080);
        assert_eq!(config.log, LogFormat::Pretty);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.body_limit, 2 * 1024 * 1024);
        assert!(!config.hsts);
        assert_eq!(config.cors_origins, None);
    }

    #[test]
    fn parses_all_fields() {
        let config = Config::parse(
            Some("127.0.0.1:9000"),
            Some("json"),
            Some("5"),
            Some("1024"),
            Some("on"),
            Some("https://a.example, https://b.example"),
        )
        .unwrap();
        assert_eq!(config.addr.port(), 9000);
        assert_eq!(config.log, LogFormat::Json);
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.body_limit, 1024);
        assert!(config.hsts);
        assert_eq!(
            config.cors_origins.as_deref(),
            Some(
                [
                    "https://a.example".to_string(),
                    "https://b.example".to_string()
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn hsts_is_lenient_boolean() {
        assert!(parse(Some("TRUE"), None).unwrap().hsts);
        assert!(parse(Some("1"), None).unwrap().hsts);
        assert!(!parse(Some("off"), None).unwrap().hsts);
    }

    #[test]
    fn cors_origins_parse_and_trim() {
        assert_eq!(
            parse(None, Some(" * ")).unwrap().cors_origins.as_deref(),
            Some(["*".to_string()].as_slice())
        );
        assert_eq!(parse(None, Some("  ")).unwrap().cors_origins, None);
        assert_eq!(parse(None, None).unwrap().cors_origins, None);
    }

    #[test]
    fn rejects_bad_addr() {
        let err = Config::parse(Some("not-an-addr"), None, None, None, None, None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_log_format() {
        let err = Config::parse(None, Some("verbose"), None, None, None, None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_timeout_and_body_limit() {
        assert!(matches!(
            Config::parse(None, None, Some("soon"), None, None, None).unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(None, None, None, Some("huge"), None, None).unwrap_err(),
            Error::Invalid(_)
        ));
    }
}
