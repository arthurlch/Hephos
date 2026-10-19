//! Hephos — the backend framework built only for coding agents.
//!
//! This meta crate is the single import surface. Use the prelude:
//!
//! ```ignore
//! use hephos::prelude::*;
//! ```
//!
//! Capabilities are feature-gated (`db`, `agent`, `mcp`) so an app only compiles
//! and only exposes what it uses.
//!
//! # Stability
//!
//! At `0.1.0` the **HTTP + database + auth** surface (`hephos_core`, `hephos_db`) is the
//! stable, reviewed API. The **`agent` and `mcp` features are pre-1.0 and unstable** —
//! their model-provider and server seams still have placeholder implementations and
//! their signatures may change in a later minor release (they are scheduled for
//! `0.5.0`/`0.6.0`). Do not treat them as frozen.

#![forbid(unsafe_code)]

pub use hephos_core::{
    App, CancellationToken, Config, ConnectionCap, ConnectionGuard, Ctx, Error, Event, Events,
    Identity, Json, LogFormat, Path, Principal, ProbeResponse, Query, RateLimitConfig, RedMetrics,
    Result, Router, StatusCode, Task, conn, http, sse, telemetry, test, ws,
};

#[cfg(feature = "db")]
pub use hephos_db as db;

#[cfg(feature = "agent")]
pub use hephos_agent as agent;

#[cfg(feature = "mcp")]
pub use hephos_mcp as mcp;

/// The one import every file starts from.
pub mod prelude {
    pub use hephos_core::{
        App, ConnectionCap, ConnectionGuard, Ctx, Error, Event, Events, Identity, Json, Path,
        Principal, Query, Result, Router, StatusCode, Task,
    };

    #[cfg(feature = "db")]
    pub use hephos_db::{Db, Tx};

    #[cfg(feature = "agent")]
    pub use hephos_agent::{Agent, Limits, Model, Tool};
}
