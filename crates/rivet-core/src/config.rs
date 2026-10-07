use std::net::SocketAddr;

use crate::error::{Error, Result};

/// Process configuration, read once at startup from the environment.
///
/// Rivet reads only what the framework itself needs. Application config
/// (database URL, provider keys, feature flags) is read by the component that
/// owns it — e.g. `Db::connect_from_env` reads `DATABASE_URL`. There is no
/// central config struct an agent must thread everywhere.
///
/// | Variable     | Default         | Meaning                       |
/// | ------------ | --------------- | ----------------------------- |
/// | `RIVET_ADDR` | `0.0.0.0:8080`  | Listen address.               |
/// | `RIVET_LOG`  | `pretty`        | `pretty` or `json` log output.|
#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,
    pub log: LogFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::parse(
            std::env::var("RIVET_ADDR").ok().as_deref(),
            std::env::var("RIVET_LOG").ok().as_deref(),
        )
    }

    /// Pure parsing of the two inputs, split from env reading so it is testable
    /// without mutating process environment (which is `unsafe` on edition 2024 and
    /// forbidden crate-wide).
    fn parse(addr: Option<&str>, log: Option<&str>) -> Result<Self> {
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

        Ok(Config { addr, log })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_unset() {
        let config = Config::parse(None, None).unwrap();
        assert_eq!(config.addr.port(), 8080);
        assert_eq!(config.log, LogFormat::Pretty);
    }

    #[test]
    fn parses_addr_and_json_log() {
        let config = Config::parse(Some("127.0.0.1:9000"), Some("json")).unwrap();
        assert_eq!(config.addr.port(), 9000);
        assert_eq!(config.log, LogFormat::Json);
    }

    #[test]
    fn rejects_bad_addr() {
        let err = Config::parse(Some("not-an-addr"), None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_bad_log_format() {
        let err = Config::parse(None, Some("verbose")).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }
}
