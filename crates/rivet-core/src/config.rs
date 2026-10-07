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
        let addr = match std::env::var("RIVET_ADDR") {
            Ok(value) => value
                .parse()
                .map_err(|_| Error::invalid(format!("RIVET_ADDR is not a socket address: {value}")))?,
            Err(_) => SocketAddr::from(([0, 0, 0, 0], 8080)),
        };

        let log = match std::env::var("RIVET_LOG").as_deref() {
            Ok("json") => LogFormat::Json,
            Ok("pretty") | Err(_) => LogFormat::Pretty,
            Ok(other) => return Err(Error::invalid(format!("RIVET_LOG must be pretty|json: {other}"))),
        };

        Ok(Config { addr, log })
    }
}
