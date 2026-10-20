//! Project scaffolding logic for `hephos new`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Rust language keywords and reserved identifiers that cannot be used as crate names.
const RESERVED_NAMES: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "test", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

/// Validates that a string is a valid Rust package name.
pub fn validate_package_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("package name cannot be empty");
    }

    if RESERVED_NAMES.contains(&name) {
        bail!("'{name}' is a reserved Rust keyword and cannot be used as a package name");
    }

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        bail!("package name cannot be empty");
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        bail!("package name must start with an ASCII letter or underscore, found '{first}'");
    }

    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '-' {
            bail!(
                "package name can only contain ASCII letters, digits, underscores, and hyphens; found invalid character '{c}'"
            );
        }
    }

    Ok(())
}

/// Scaffolds a complete Hephos application conforming to the AGENTS.md layout.
pub fn scaffold_app(name: &str, target_dir: &Path, local_hephos_path: Option<&Path>) -> Result<()> {
    validate_package_name(name)?;

    if target_dir.exists() {
        let mut entries = fs::read_dir(target_dir)
            .with_context(|| format!("failed to read directory {}", target_dir.display()))?;
        if entries.next().is_some() {
            bail!(
                "destination directory '{}' already exists and is not empty",
                target_dir.display()
            );
        }
    } else {
        fs::create_dir_all(target_dir)
            .with_context(|| format!("failed to create directory {}", target_dir.display()))?;
    }

    let src_dir = target_dir.join("src");
    let api_dir = src_dir.join("api");
    let domain_dir = src_dir.join("domain");
    let services_dir = src_dir.join("services");

    fs::create_dir_all(&api_dir)?;
    fs::create_dir_all(&domain_dir)?;
    fs::create_dir_all(&services_dir)?;

    let hephos_dep = match local_hephos_path {
        Some(path) => {
            let path_str = path.display().to_string().replace('\\', "/");
            format!("hephos = {{ path = \"{path_str}\" }}")
        }
        None => "hephos = { version = \"1.0.0\" }".to_string(),
    };

    // 1. Cargo.toml
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
{hephos_dep}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#
    );
    fs::write(target_dir.join("Cargo.toml"), cargo_toml)?;

    // 2. .gitignore
    let gitignore = "/target\nCargo.lock\n";
    fs::write(target_dir.join(".gitignore"), gitignore)?;

    // 3. src/main.rs
    let main_rs = r#"mod api;
mod domain;
mod services;
mod state;
#[cfg(test)]
mod tests;

use hephos::prelude::*;

use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState::init().await?;
    App::new(state).routes(api::routes()).run().await
}
"#;
    fs::write(src_dir.join("main.rs"), main_rs)?;

    // 4. src/state.rs
    let state_rs = r#"use hephos::prelude::*;

use crate::services::hello::HelloService;

pub type Ctx = hephos::Ctx<AppState>;

#[derive(Clone)]
pub struct AppState {
    pub hello: HelloService,
}

impl AppState {
    pub async fn init() -> Result<Self> {
        Ok(Self {
            hello: HelloService::new(),
        })
    }
}
"#;
    fs::write(src_dir.join("state.rs"), state_rs)?;

    // 5. src/api.rs
    let api_rs = r#"pub mod hello;

use hephos::prelude::*;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().merge(hello::routes())
}
"#;
    fs::write(src_dir.join("api.rs"), api_rs)?;

    // 6. src/api/hello.rs
    let api_hello_rs = r#"use hephos::prelude::*;

use crate::domain::hello::Greeting;
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new().get("/hello", get_hello)
}

pub async fn get_hello(ctx: Ctx) -> Result<Json<Greeting>> {
    let greeting = ctx.state().hello.greet().await?;
    Ok(Json(greeting))
}
"#;
    fs::write(api_dir.join("hello.rs"), api_hello_rs)?;

    // 7. src/domain.rs
    let domain_rs = "pub mod hello;\n";
    fs::write(src_dir.join("domain.rs"), domain_rs)?;

    // 8. src/domain/hello.rs
    let domain_hello_rs = r#"use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Greeting {
    pub message: String,
}
"#;
    fs::write(domain_dir.join("hello.rs"), domain_hello_rs)?;

    // 9. src/services.rs
    let services_rs = "pub mod hello;\n";
    fs::write(src_dir.join("services.rs"), services_rs)?;

    // 10. src/services/hello.rs
    let services_hello_rs = r#"use hephos::prelude::*;

use crate::domain::hello::Greeting;

#[derive(Clone, Default)]
pub struct HelloService;

impl HelloService {
    pub fn new() -> Self {
        Self
    }

    pub async fn greet(&self) -> Result<Greeting> {
        Ok(Greeting {
            message: "Hello from Hephos!".to_string(),
        })
    }
}
"#;
    fs::write(services_dir.join("hello.rs"), services_hello_rs)?;

    // 11. src/tests.rs
    let tests_rs = r#"use hephos::prelude::*;
use hephos::test::TestClient;

use crate::domain::hello::Greeting;
use crate::state::AppState;

#[tokio::test]
async fn routes_build() {
    // Canary test per AGENTS.md § 10: router composition must build cleanly without panics.
    let _ = crate::api::routes();
}

#[tokio::test]
async fn get_hello_returns_200_and_greeting() {
    let state = AppState::init().await.expect("app state init");
    let client = TestClient::new(crate::api::routes(), state);
    let res = client.get("/hello").await;
    assert_eq!(res.status(), 200);
    let body = res.json::<Greeting>();
    assert_eq!(body.message, "Hello from Hephos!");
}
"#;
    fs::write(src_dir.join("tests.rs"), tests_rs)?;

    Ok(())
}
