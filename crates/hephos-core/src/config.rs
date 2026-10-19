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
/// | `HEPHOS_HSTS`             | `off`          | Send `Strict-Transport-Security` (only behind TLS). |
/// | `HEPHOS_CORS_ORIGINS`     | *(unset)*      | Comma-separated allowed origins, or `*`. Unset = no CORS. |
/// | `HEPHOS_RATE_LIMIT_RPS`   | *(unset)*      | Quota replenishment in requests/sec per IP. Unset = disabled. |
/// | `HEPHOS_RATE_LIMIT_BURST` | *(same as RPS)*| Maximum burst capacity per IP. |
/// | `HEPHOS_CONCURRENCY_LIMIT`| *(unset)*      | Max concurrent in-flight requests before load shedding (503). |
/// | `HEPHOS_MAX_CONNECTIONS`  | *(unset)*      | Max concurrent active connections before rejection (503). |
/// | `HEPHOS_OTLP_ENDPOINT`    | *(unset)*      | OTLP HTTP collector endpoint (e.g. `http://localhost:4318`). |
/// | `HEPHOS_TRACES`           | `off`          | Export distributed traces via OTLP (beta upstream, gated). |
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
    /// Rate limit configuration. `None` disables rate limiting (the default).
    pub rate_limit: Option<RateLimitConfig>,
    /// Maximum concurrent in-flight requests before load shedding triggers. `None`
    /// disables concurrency limits and load shedding (the default).
    pub concurrency_limit: Option<usize>,
    /// Maximum concurrent active connections allowed before new connections are rejected.
    /// `None` disables the global connection cap (the default).
    pub max_connections: Option<usize>,
    /// OpenTelemetry collector endpoint for OTLP export (`HEPHOS_OTLP_ENDPOINT`).
    pub otlp_endpoint: Option<String>,
    /// Whether OpenTelemetry trace export is enabled (`HEPHOS_TRACES`).
    pub traces_enabled: bool,
}

/// Rate limiting configuration (`HEPHOS_RATE_LIMIT_RPS`, `HEPHOS_RATE_LIMIT_BURST`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Quota replenishment rate in requests per second.
    pub rps: u64,
    /// Maximum burst capacity allowed before rate limiting triggers.
    pub burst: u32,
}

impl RateLimitConfig {
    pub fn new(rps: u64, burst: u32) -> Self {
        RateLimitConfig { rps, burst }
    }
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

#[derive(Default)]
struct RawEnv<'a> {
    addr: Option<&'a str>,
    log: Option<&'a str>,
    timeout_secs: Option<&'a str>,
    body_limit: Option<&'a str>,
    hsts: Option<&'a str>,
    cors_origins: Option<&'a str>,
    rate_limit_rps: Option<&'a str>,
    rate_limit_burst: Option<&'a str>,
    concurrency_limit: Option<&'a str>,
    max_connections: Option<&'a str>,
    otlp_endpoint: Option<&'a str>,
    traces: Option<&'a str>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let addr = std::env::var("HEPHOS_ADDR").ok();
        let log = std::env::var("HEPHOS_LOG").ok();
        let timeout_secs = std::env::var("HEPHOS_TIMEOUT_SECS").ok();
        let body_limit = std::env::var("HEPHOS_BODY_LIMIT").ok();
        let hsts = std::env::var("HEPHOS_HSTS").ok();
        let cors_origins = std::env::var("HEPHOS_CORS_ORIGINS").ok();
        let rate_limit_rps = std::env::var("HEPHOS_RATE_LIMIT_RPS").ok();
        let rate_limit_burst = std::env::var("HEPHOS_RATE_LIMIT_BURST").ok();
        let concurrency_limit = std::env::var("HEPHOS_CONCURRENCY_LIMIT").ok();
        let max_connections = std::env::var("HEPHOS_MAX_CONNECTIONS").ok();
        let otlp_endpoint = std::env::var("HEPHOS_OTLP_ENDPOINT")
            .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
            .ok();
        let traces = std::env::var("HEPHOS_TRACES")
            .or_else(|_| std::env::var("HEPHOS_OTEL_TRACES"))
            .ok();

        Self::parse(RawEnv {
            addr: addr.as_deref(),
            log: log.as_deref(),
            timeout_secs: timeout_secs.as_deref(),
            body_limit: body_limit.as_deref(),
            hsts: hsts.as_deref(),
            cors_origins: cors_origins.as_deref(),
            rate_limit_rps: rate_limit_rps.as_deref(),
            rate_limit_burst: rate_limit_burst.as_deref(),
            concurrency_limit: concurrency_limit.as_deref(),
            max_connections: max_connections.as_deref(),
            otlp_endpoint: otlp_endpoint.as_deref(),
            traces: traces.as_deref(),
        })
    }

    /// Pure parsing of the inputs, split from env reading so it is testable without
    /// mutating process environment (which is `unsafe` on edition 2024 and forbidden
    /// crate-wide).
    fn parse(env: RawEnv<'_>) -> Result<Self> {
        let addr = match env.addr {
            Some(value) => value.parse().map_err(|_| {
                Error::invalid(format!("HEPHOS_ADDR is not a socket address: {value}"))
            })?,
            None => SocketAddr::from(([0, 0, 0, 0], 8080)),
        };

        let log = match env.log {
            Some("json") => LogFormat::Json,
            Some("pretty") | None => LogFormat::Pretty,
            Some(other) => {
                return Err(Error::invalid(format!(
                    "HEPHOS_LOG must be pretty|json: {other}"
                )));
            }
        };

        let timeout = match env.timeout_secs {
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

        let body_limit = match env.body_limit {
            Some(value) => value.parse::<usize>().map_err(|_| {
                Error::invalid(format!("HEPHOS_BODY_LIMIT must be a byte count: {value}"))
            })?,
            None => DEFAULT_BODY_LIMIT,
        };

        let hsts = matches!(
            env.hsts.map(str::to_ascii_lowercase).as_deref(),
            Some("1" | "on" | "true" | "yes")
        );

        let cors_origins = env.cors_origins.and_then(|value| {
            let origins: Vec<String> = value
                .split(',')
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .map(str::to_owned)
                .collect();
            (!origins.is_empty()).then_some(origins)
        });

        let rate_limit = match (env.rate_limit_rps, env.rate_limit_burst) {
            (Some(rps_str), burst_opt) => {
                let rps = rps_str.parse::<u64>().map_err(|_| {
                    Error::invalid(format!(
                        "HEPHOS_RATE_LIMIT_RPS must be a positive integer: {rps_str}"
                    ))
                })?;
                if rps == 0 {
                    return Err(Error::invalid(
                        "HEPHOS_RATE_LIMIT_RPS must be greater than zero",
                    ));
                }
                let burst = match burst_opt {
                    Some(burst_str) => {
                        let burst = burst_str.parse::<u32>().map_err(|_| {
                            Error::invalid(format!(
                                "HEPHOS_RATE_LIMIT_BURST must be a positive integer: {burst_str}"
                            ))
                        })?;
                        if burst == 0 {
                            return Err(Error::invalid(
                                "HEPHOS_RATE_LIMIT_BURST must be greater than zero",
                            ));
                        }
                        burst
                    }
                    None => u32::try_from(rps).unwrap_or(u32::MAX),
                };
                Some(RateLimitConfig { rps, burst })
            }
            (None, Some(burst_str)) => {
                return Err(Error::invalid(format!(
                    "HEPHOS_RATE_LIMIT_BURST is set ({burst_str}) without HEPHOS_RATE_LIMIT_RPS"
                )));
            }
            (None, None) => None,
        };

        let concurrency_limit = match env.concurrency_limit {
            Some(value) => {
                let limit = value.parse::<usize>().map_err(|_| {
                    Error::invalid(format!(
                        "HEPHOS_CONCURRENCY_LIMIT must be a positive integer: {value}"
                    ))
                })?;
                if limit == 0 {
                    return Err(Error::invalid(
                        "HEPHOS_CONCURRENCY_LIMIT must be greater than zero",
                    ));
                }
                Some(limit)
            }
            None => None,
        };

        let max_connections = match env.max_connections {
            Some(value) => {
                let limit = value.parse::<usize>().map_err(|_| {
                    Error::invalid(format!(
                        "HEPHOS_MAX_CONNECTIONS must be a positive integer: {value}"
                    ))
                })?;
                if limit == 0 {
                    return Err(Error::invalid(
                        "HEPHOS_MAX_CONNECTIONS must be greater than zero",
                    ));
                }
                Some(limit)
            }
            None => None,
        };

        let otlp_endpoint = env.otlp_endpoint.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });

        let traces_enabled = matches!(
            env.traces.map(str::to_ascii_lowercase).as_deref(),
            Some("1" | "on" | "true" | "yes")
        );

        Ok(Config {
            addr,
            log,
            timeout,
            body_limit,
            hsts,
            cors_origins,
            rate_limit,
            concurrency_limit,
            max_connections,
            otlp_endpoint,
            traces_enabled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(hsts: Option<&str>, cors: Option<&str>) -> Result<Config> {
        Config::parse(RawEnv {
            hsts,
            cors_origins: cors,
            ..Default::default()
        })
    }

    #[test]
    fn defaults_when_unset() {
        let config = Config::parse(RawEnv::default()).unwrap();
        assert_eq!(config.addr.port(), 8080);
        assert_eq!(config.log, LogFormat::Pretty);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.body_limit, 2 * 1024 * 1024);
        assert!(!config.hsts);
        assert_eq!(config.cors_origins, None);
        assert_eq!(config.rate_limit, None);
        assert_eq!(config.concurrency_limit, None);
        assert_eq!(config.otlp_endpoint, None);
        assert!(!config.traces_enabled);
    }

    #[test]
    fn parses_all_fields() {
        let config = Config::parse(RawEnv {
            addr: Some("127.0.0.1:9000"),
            log: Some("json"),
            timeout_secs: Some("5"),
            body_limit: Some("1024"),
            hsts: Some("on"),
            cors_origins: Some("https://a.example, https://b.example"),
            rate_limit_rps: Some("100"),
            rate_limit_burst: Some("250"),
            concurrency_limit: Some("64"),
            max_connections: Some("128"),
            otlp_endpoint: Some("http://localhost:4318"),
            traces: Some("true"),
        })
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
        assert_eq!(
            config.rate_limit,
            Some(RateLimitConfig {
                rps: 100,
                burst: 250
            })
        );
        assert_eq!(config.concurrency_limit, Some(64));
        assert_eq!(config.max_connections, Some(128));
        assert_eq!(
            config.otlp_endpoint.as_deref(),
            Some("http://localhost:4318")
        );
        assert!(config.traces_enabled);
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
    fn rate_limit_burst_defaults_to_rps() {
        let config = Config::parse(RawEnv {
            rate_limit_rps: Some("50"),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            config.rate_limit,
            Some(RateLimitConfig { rps: 50, burst: 50 })
        );
    }

    #[test]
    fn rate_limit_rejects_invalid_values() {
        assert!(matches!(
            Config::parse(RawEnv {
                rate_limit_rps: Some("0"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(RawEnv {
                rate_limit_rps: Some("abc"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(RawEnv {
                rate_limit_rps: Some("10"),
                rate_limit_burst: Some("0"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(RawEnv {
                rate_limit_burst: Some("10"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
    }

    #[test]
    fn concurrency_limit_rejects_invalid_values() {
        assert!(matches!(
            Config::parse(RawEnv {
                concurrency_limit: Some("0"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(RawEnv {
                concurrency_limit: Some("invalid"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
    }

    #[test]
    fn rejects_bad_addr() {
        let err = Config::parse(RawEnv {
            addr: Some("not-an-addr"),
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_log_format() {
        let err = Config::parse(RawEnv {
            log: Some("verbose"),
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_timeout_and_body_limit() {
        assert!(matches!(
            Config::parse(RawEnv {
                timeout_secs: Some("soon"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(RawEnv {
                body_limit: Some("huge"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
    }

    #[test]
    fn rate_limit_config_constructor() {
        let rlc = RateLimitConfig::new(20, 40);
        assert_eq!(rlc.rps, 20);
        assert_eq!(rlc.burst, 40);
        let cloned = rlc;
        assert_eq!(cloned, rlc);
    }

    #[test]
    fn log_format_traits() {
        assert_eq!(LogFormat::Pretty, LogFormat::Pretty);
        assert_ne!(LogFormat::Pretty, LogFormat::Json);
        assert_eq!(format!("{:?}", LogFormat::Json), "Json");
        assert_eq!(format!("{:?}", LogFormat::Pretty), "Pretty");
    }

    #[test]
    fn parses_valid_max_connections() {
        let config = Config::parse(RawEnv {
            max_connections: Some("500"),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(config.max_connections, Some(500));
    }

    #[test]
    fn rejects_invalid_max_connections() {
        assert!(matches!(
            Config::parse(RawEnv {
                max_connections: Some("zero"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
        assert!(matches!(
            Config::parse(RawEnv {
                max_connections: Some("0"),
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid(_)
        ));
    }
}
