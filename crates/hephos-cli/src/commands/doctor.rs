use std::path::Path;
use std::process::Command;

use anyhow::Result;

/// A single diagnostic report item.
#[derive(Debug, Clone)]
pub struct DiagnosticItem {
    pub name: &'static str,
    pub status: DiagnosticStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticStatus {
    Ok,
    Warning,
    Error,
}

/// Run environment diagnostics for Hephos development.
pub fn run_doctor(project_root: &Path) -> Result<Vec<DiagnosticItem>> {
    let mut diagnostics = Vec::new();

    // 1. Check rustc version
    match Command::new("rustc").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let ver_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            diagnostics.push(DiagnosticItem {
                name: "Rust compiler (rustc)",
                status: DiagnosticStatus::Ok,
                detail: ver_str,
            });
        }
        _ => {
            diagnostics.push(DiagnosticItem {
                name: "Rust compiler (rustc)",
                status: DiagnosticStatus::Error,
                detail: "rustc not found in PATH; install via https://rustup.rs".to_string(),
            });
        }
    }

    // 2. Check cargo
    match Command::new("cargo").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let ver_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            diagnostics.push(DiagnosticItem {
                name: "Cargo package manager",
                status: DiagnosticStatus::Ok,
                detail: ver_str,
            });
        }
        _ => {
            diagnostics.push(DiagnosticItem {
                name: "Cargo package manager",
                status: DiagnosticStatus::Error,
                detail: "cargo not found in PATH".to_string(),
            });
        }
    }

    // 3. Check rustfmt
    let rustfmt_installed = Command::new("rustfmt")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if rustfmt_installed {
        diagnostics.push(DiagnosticItem {
            name: "Rust formatter (rustfmt)",
            status: DiagnosticStatus::Ok,
            detail: "rustfmt is installed and ready".to_string(),
        });
    } else {
        diagnostics.push(DiagnosticItem {
            name: "Rust formatter (rustfmt)",
            status: DiagnosticStatus::Warning,
            detail: "rustfmt not found; install with: rustup component add rustfmt".to_string(),
        });
    }

    // 4. Check clippy
    let clippy_installed = Command::new("cargo")
        .args(["clippy", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if clippy_installed {
        diagnostics.push(DiagnosticItem {
            name: "Rust linter (clippy)",
            status: DiagnosticStatus::Ok,
            detail: "cargo-clippy is installed and ready".to_string(),
        });
    } else {
        diagnostics.push(DiagnosticItem {
            name: "Rust linter (clippy)",
            status: DiagnosticStatus::Warning,
            detail: "clippy not found; install with: rustup component add clippy".to_string(),
        });
    }

    // 5. Check sqlx-cli
    let sqlx_installed = Command::new("cargo")
        .args(["sqlx", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if sqlx_installed {
        diagnostics.push(DiagnosticItem {
            name: "SQLx CLI",
            status: DiagnosticStatus::Ok,
            detail: "cargo-sqlx is installed and ready".to_string(),
        });
    } else {
        diagnostics.push(DiagnosticItem {
            name: "SQLx CLI",
            status: DiagnosticStatus::Warning,
            detail: "cargo-sqlx not found. Install with: cargo install sqlx-cli --no-default-features --features postgres".to_string(),
        });
    }

    // 6. Check DATABASE_URL
    let db_url = std::env::var("DATABASE_URL").ok();
    let env_file = project_root.join(".env");
    let env_db_url = if env_file.exists() {
        std::fs::read_to_string(&env_file)
            .ok()
            .and_then(|contents| {
                contents.lines().find_map(|line| {
                    let trimmed = line.trim();
                    if trimmed.starts_with('#') {
                        return None;
                    }
                    let (k, v) = trimmed.split_once('=')?;
                    if k.trim() == "DATABASE_URL" {
                        Some(v.trim().trim_matches('"').trim_matches('\'').to_string())
                    } else {
                        None
                    }
                })
            })
    } else {
        None
    };

    if let Some(url) = db_url {
        let masked = mask_db_url(&url);
        diagnostics.push(DiagnosticItem {
            name: "DATABASE_URL environment variable",
            status: DiagnosticStatus::Ok,
            detail: format!("Configured in environment ({masked})"),
        });
    } else if let Some(url) = env_db_url {
        let masked = mask_db_url(&url);
        diagnostics.push(DiagnosticItem {
            name: "DATABASE_URL environment variable",
            status: DiagnosticStatus::Ok,
            detail: format!("Configured in .env ({masked})"),
        });
    } else if env_file.exists() {
        diagnostics.push(DiagnosticItem {
            name: "DATABASE_URL environment variable",
            status: DiagnosticStatus::Warning,
            detail: ".env file found, but DATABASE_URL is not defined in it".to_string(),
        });
    } else {
        diagnostics.push(DiagnosticItem {
            name: "DATABASE_URL environment variable",
            status: DiagnosticStatus::Warning,
            detail: "DATABASE_URL not set in environment and no .env file found".to_string(),
        });
    }

    // 5. Check Git repository
    let is_git = project_root.ancestors().any(|p| p.join(".git").exists());
    if is_git {
        diagnostics.push(DiagnosticItem {
            name: "Version control (git)",
            status: DiagnosticStatus::Ok,
            detail: "Git repository initialized".to_string(),
        });
    } else {
        diagnostics.push(DiagnosticItem {
            name: "Version control (git)",
            status: DiagnosticStatus::Warning,
            detail: "No .git directory found".to_string(),
        });
    }

    // 6. Check canonical project structure (if src/ exists)
    let src_dir = project_root.join("src");
    if src_dir.exists() {
        let mut missing_dirs = Vec::new();
        for dir_name in &["api", "services", "domain"] {
            if !src_dir.join(dir_name).exists() && !src_dir.join(format!("{dir_name}.rs")).exists()
            {
                missing_dirs.push(*dir_name);
            }
        }
        if missing_dirs.is_empty() {
            diagnostics.push(DiagnosticItem {
                name: "AGENTS.md project structure",
                status: DiagnosticStatus::Ok,
                detail: "Standard directories (api, services, domain) present".to_string(),
            });
        } else {
            diagnostics.push(DiagnosticItem {
                name: "AGENTS.md project structure",
                status: DiagnosticStatus::Warning,
                detail: format!("Missing canonical modules: {}", missing_dirs.join(", ")),
            });
        }
    }

    Ok(diagnostics)
}

fn mask_db_url(url: &str) -> String {
    if let Some((proto, rest)) = url.split_once("://") {
        if let Some((cred, host_path)) = rest.split_once('@') {
            let user = cred.split_once(':').map(|(u, _)| u).unwrap_or(cred);
            return format!("{proto}://{user}:****@{host_path}");
        }
    }
    "postgres://****".to_string()
}
