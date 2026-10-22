//! Hephos CLI library crate.
//!
//! Provides the core commands for the `hephos` executable:
//! - [`scaffold::scaffold_app`]: generates a complete canonical Hephos app skeleton.
//! - [`routes::scan_routes`]: statically scans and reflects application routes.
//! - [`check::check_project`]: validates AGENTS.md architectural layering rules.
//! - [`doctor::run_doctor`]: inspects developer environment and dependencies.
//! - [`generate::generate_resource`]: scaffolds canonical domain/repo/service/api resources.

#![forbid(unsafe_code)]

pub mod commands;

pub use commands::check::{ArchViolation, check_project};
pub use commands::doctor::{DiagnosticItem, DiagnosticStatus, run_doctor};
pub use commands::generate::{
    generate_migration, generate_resource, validate_migration_name, validate_resource_name,
};
pub use commands::routes::{
    DiscoveredRoute, format_json, format_table, generate_openapi, scan_routes,
};
pub use commands::scaffold::scaffold_app;
pub use commands::{check, doctor, generate, routes, scaffold};
