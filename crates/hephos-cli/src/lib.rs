//! Hephos CLI library crate.
//!
//! Provides the core commands for the `hephos` executable:
//! - [`scaffold::scaffold_app`]: generates a complete canonical Hephos app skeleton.
//! - [`routes::scan_routes`]: statically scans and reflects application routes.
//! - [`check::check_project`]: validates AGENTS.md architectural layering rules.
//! - [`doctor::run_doctor`]: inspects developer environment and dependencies.
//! - [`generate::generate_resource`]: scaffolds canonical domain/repo/service/api resources.

pub mod check;
pub mod doctor;
pub mod generate;
pub mod routes;
pub mod scaffold;

pub use check::{ArchViolation, check_project};
pub use doctor::{DiagnosticItem, DiagnosticStatus, run_doctor};
pub use generate::{generate_migration, generate_resource};
pub use routes::{DiscoveredRoute, format_json, format_table, generate_openapi, scan_routes};
pub use scaffold::scaffold_app;
