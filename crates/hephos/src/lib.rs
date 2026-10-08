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

#![forbid(unsafe_code)]

pub use hephos_core::{
    App, CancellationToken, Config, Ctx, Error, Event, Events, Identity, Json, LogFormat, Path,
    Principal, Query, Result, Router, Task, sse, test, ws,
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
        App, Ctx, Error, Event, Events, Identity, Json, Path, Principal, Query, Result, Router,
        Task,
    };

    #[cfg(feature = "db")]
    pub use hephos_db::{Db, Tx};

    #[cfg(feature = "agent")]
    pub use hephos_agent::{Agent, Limits, Model, Tool};
}
