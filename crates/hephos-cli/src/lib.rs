//! Hephos CLI library crate.
//!
//! Provides the core commands for the `hephos` executable:
//! - [`scaffold::scaffold_app`]: generates a complete canonical Hephos app skeleton.
//! - [`routes::scan_routes`]: statically scans and reflects application routes.

pub mod routes;
pub mod scaffold;

pub use routes::{DiscoveredRoute, format_json, format_table, generate_openapi, scan_routes};
pub use scaffold::scaffold_app;
