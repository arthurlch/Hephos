use std::fs;
use std::path::Path;

use anyhow::{Result, bail};

/// Rust language keywords and reserved identifiers that cannot be used as module or struct names.
const RESERVED_NAMES: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "test", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

/// Validates that a resource name is safe from path traversal and valid as a Rust identifier.
pub fn validate_resource_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("resource name cannot be empty");
    }

    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        bail!("resource name cannot contain path separators, '..', or null bytes");
    }

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        bail!("resource name cannot be empty");
    };

    if !first.is_ascii_lowercase() && first != '_' {
        bail!(
            "resource name must start with an ASCII lowercase letter or underscore, found '{first}'"
        );
    }

    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' {
            bail!(
                "resource name can only contain ASCII alphanumeric characters and underscores, found invalid character '{c}'"
            );
        }
    }

    if RESERVED_NAMES.contains(&name) {
        bail!("'{name}' is a reserved Rust keyword and cannot be used as a resource name");
    }

    let singular = name.trim_end_matches('s');
    if singular.is_empty() {
        bail!("resource name cannot consist only of 's'");
    }

    if RESERVED_NAMES.contains(&singular) {
        bail!(
            "singular form '{singular}' is a reserved Rust keyword and cannot be used as a resource name"
        );
    }

    Ok(())
}

/// Validates that a migration name is safe from path traversal and SQL injection.
pub fn validate_migration_name(name: &str) -> Result<String> {
    if name.is_empty() {
        bail!("migration name cannot be empty");
    }

    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        bail!("migration name cannot contain path separators, '..', or null bytes");
    }

    let clean_name = name.trim().to_lowercase().replace([' ', '-'], "_");
    if clean_name.is_empty() {
        bail!("migration name cannot be empty");
    }

    let mut chars = clean_name.chars();
    let Some(first) = chars.next() else {
        bail!("migration name cannot be empty");
    };

    if !first.is_ascii_alphanumeric() && first != '_' {
        bail!(
            "migration name must start with an ASCII letter, digit, or underscore, found '{first}'"
        );
    }

    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' {
            bail!(
                "migration name can only contain ASCII alphanumeric characters and underscores, found invalid character '{c}'"
            );
        }
    }

    let table_name = clean_name
        .trim_start_matches("create_")
        .trim_end_matches("_table");

    if table_name.is_empty() {
        bail!("migration table name cannot be empty");
    }

    for c in table_name.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' {
            bail!(
                "derived table name '{table_name}' contains invalid SQL identifier character '{c}'"
            );
        }
    }

    Ok(clean_name)
}

/// Generate canonical domain, repo, service, and api scaffold files for a resource.
pub fn generate_resource(project_root: &Path, resource: &str) -> Result<Vec<String>> {
    let raw_name = resource.trim().to_lowercase();
    validate_resource_name(&raw_name)?;

    let src_dir = project_root.join("src");
    if !src_dir.exists() {
        bail!(
            "project directory '{}' does not contain a 'src' directory",
            project_root.display()
        );
    }

    let singular = raw_name.trim_end_matches('s');
    let plural = format!("{singular}s");
    let type_name = to_pascal_case(singular);

    let mut created_files = Vec::new();

    // 1. domain/<singular>.rs
    let domain_dir = src_dir.join("domain");
    fs::create_dir_all(&domain_dir)?;
    let domain_file = domain_dir.join(format!("{singular}.rs"));
    if !domain_file.exists() {
        let domain_content = format!(
            r#"use hephos::prelude::*;
use serde::{{Deserialize, Serialize}};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct {type_name} {{
    pub id: Uuid,
    pub name: String,
}}

#[derive(Debug, Clone, Deserialize)]
pub struct Create{type_name} {{
    pub name: String,
}}
"#
        );
        fs::write(&domain_file, domain_content)?;
        created_files.push(format!("src/domain/{singular}.rs"));
    }

    // Wire into domain.rs if present
    let domain_mod = src_dir.join("domain.rs");
    if domain_mod.exists() {
        let line = format!("pub mod {singular};");
        let content = fs::read_to_string(&domain_mod)?;
        if !content.contains(&line) {
            let mut new_c = content;
            if !new_c.ends_with('\n') {
                new_c.push('\n');
            }
            new_c.push_str(&line);
            new_c.push('\n');
            fs::write(&domain_mod, new_c)?;
            created_files.push(format!("src/domain.rs (declared pub mod {singular})"));
        }
    }

    // 2. repos/<plural>.rs
    let repos_dir = src_dir.join("repos");
    fs::create_dir_all(&repos_dir)?;
    let repo_file = repos_dir.join(format!("{plural}.rs"));
    if !repo_file.exists() {
        let repo_content = format!(
            r#"use hephos::prelude::*;
use crate::domain::{singular}::{{{type_name}, Create{type_name}}};

pub struct {type_name}Repo;

impl {type_name}Repo {{
    pub async fn find(pool: &sqlx::PgPool, id: Uuid) -> Result<Option<{type_name}>> {{
        // Canonical SQL-only repository method
        let _ = (pool, id);
        Ok(None)
    }}

    pub async fn create(pool: &sqlx::PgPool, input: Create{type_name}) -> Result<{type_name}> {{
        let _ = pool;
        Ok({type_name} {{
            id: Uuid::new_v4(),
            name: input.name,
        }})
    }}
}}
"#
        );
        fs::write(&repo_file, repo_content)?;
        created_files.push(format!("src/repos/{plural}.rs"));
    }

    // Wire into repos.rs (create if absent)
    let repos_mod = src_dir.join("repos.rs");
    let repo_mod_line = format!("pub mod {plural};");
    if !repos_mod.exists() {
        fs::write(&repos_mod, format!("{repo_mod_line}\n"))?;
        created_files.push(format!("src/repos.rs (created with pub mod {plural})"));
    } else {
        let content = fs::read_to_string(&repos_mod)?;
        if !content.contains(&repo_mod_line) {
            let mut new_c = content;
            if !new_c.ends_with('\n') {
                new_c.push('\n');
            }
            new_c.push_str(&repo_mod_line);
            new_c.push('\n');
            fs::write(&repos_mod, new_c)?;
            created_files.push(format!("src/repos.rs (declared pub mod {plural})"));
        }
    }

    // Wire mod repos into main.rs if present
    let main_file = src_dir.join("main.rs");
    if main_file.exists() {
        let content = fs::read_to_string(&main_file)?;
        if !content.contains("mod repos;") {
            let new_c = if let Some(idx) = content.find("mod services;") {
                let (before, after) = content.split_at(idx);
                format!("{before}mod repos;\n{after}")
            } else {
                format!("mod repos;\n{content}")
            };
            fs::write(&main_file, new_c)?;
            created_files.push("src/main.rs (declared mod repos)".to_string());
        }
    }

    // 3. services/<plural>.rs
    let services_dir = src_dir.join("services");
    fs::create_dir_all(&services_dir)?;
    let service_file = services_dir.join(format!("{plural}.rs"));
    if !service_file.exists() {
        let service_content = format!(
            r#"use hephos::prelude::*;
use crate::state::Ctx;
use crate::domain::{singular}::{{{type_name}, Create{type_name}}};
use crate::repos::{plural}::{type_name}Repo;

#[derive(Clone)]
pub struct {type_name}Service {{
    db: Db,
}}

impl {type_name}Service {{
    pub fn new(db: Db) -> Self {{
        Self {{ db }}
    }}

    pub async fn get(&self, _ctx: &Ctx, id: Uuid) -> Result<{type_name}> {{
        match {type_name}Repo::find(self.db.pool(), id).await? {{
            Some(item) => Ok(item),
            None => Err(Error::not_found("{singular}")),
        }}
    }}

    pub async fn create(&self, _ctx: &Ctx, input: Create{type_name}) -> Result<{type_name}> {{
        if input.name.trim().is_empty() {{
            return Err(Error::invalid("name cannot be empty"));
        }}
        {type_name}Repo::create(self.db.pool(), input).await
    }}
}}
"#
        );
        fs::write(&service_file, service_content)?;
        created_files.push(format!("src/services/{plural}.rs"));
    }

    // Wire into services.rs if present
    let services_mod = src_dir.join("services.rs");
    if services_mod.exists() {
        let line = format!("pub mod {plural};");
        let content = fs::read_to_string(&services_mod)?;
        if !content.contains(&line) {
            let mut new_c = content;
            if !new_c.ends_with('\n') {
                new_c.push('\n');
            }
            new_c.push_str(&line);
            new_c.push('\n');
            fs::write(&services_mod, new_c)?;
            created_files.push(format!("src/services.rs (declared pub mod {plural})"));
        }
    }

    // 4. api/<plural>.rs
    let api_dir = src_dir.join("api");
    fs::create_dir_all(&api_dir)?;
    let api_file = api_dir.join(format!("{plural}.rs"));
    if !api_file.exists() {
        let api_content = format!(
            r#"use hephos::prelude::*;
use crate::state::{{AppState, Ctx}};
use crate::domain::{singular}::{{{type_name}, Create{type_name}}};

pub fn routes() -> Router<AppState> {{
    Router::new()
        .get("/{plural}/{{id}}", get_one)
        .post("/{plural}", create_one)
}}

pub async fn get_one(ctx: Ctx, Path(id): Path<Uuid>) -> Result<Json<{type_name}>> {{
    let item = ctx.state().{plural}.get(&ctx, id).await?;
    Ok(Json(item))
}}

pub async fn create_one(ctx: Ctx, Json(input): Json<Create{type_name}>) -> Result<Json<{type_name}>> {{
    let item = ctx.state().{plural}.create(&ctx, input).await?;
    Ok(Json(item))
}}
"#
        );
        fs::write(&api_file, api_content)?;
        created_files.push(format!("src/api/{plural}.rs"));
    }

    // Wire into api.rs if present
    let api_mod = src_dir.join("api.rs");
    if api_mod.exists() {
        let line = format!("pub mod {plural};");
        let content = fs::read_to_string(&api_mod)?;
        let mut new_c = content;
        if !new_c.contains(&line) {
            new_c = format!("{line}\n{new_c}");
            created_files.push(format!("src/api.rs (declared pub mod {plural})"));
        }
        let merged_c = wire_api_routes(&new_c, &plural);
        if merged_c != new_c {
            new_c = merged_c;
            created_files.push(format!("src/api.rs (merged {plural}::routes())"));
        }
        fs::write(&api_mod, new_c)?;
    }

    // Wire into state.rs if present
    let state_file = src_dir.join("state.rs");
    if state_file.exists() {
        let content = fs::read_to_string(&state_file)?;
        let (new_c, updated) = wire_state_service(&content, &plural, &type_name);
        if updated {
            fs::write(&state_file, new_c)?;
            created_files.push(format!(
                "src/state.rs (registered {type_name}Service into AppState)"
            ));
        }
    }

    // Wire features = ["db"] and sqlx/uuid into Cargo.toml if absent
    let cargo_file = project_root.join("Cargo.toml");
    if cargo_file.exists() {
        let cargo_content = fs::read_to_string(&cargo_file)?;
        if !cargo_content.contains("features = [\"db\"]")
            && !cargo_content.contains("features=[\"db\"]")
        {
            let mut updated_cargo = cargo_content.clone();
            if cargo_content.contains("hephos = { version = \"1.0.0\" }") {
                updated_cargo = cargo_content.replace(
                    "hephos = { version = \"1.0.0\" }",
                    "hephos = { version = \"1.0.0\", features = [\"db\"] }\nsqlx = { version = \"0.8\", default-features = false, features = [\"runtime-tokio\", \"tls-rustls\", \"postgres\", \"macros\", \"uuid\", \"chrono\"] }\nuuid = { version = \"1\", features = [\"v4\", \"serde\"] }",
                );
            } else if let Some(idx) = cargo_content.find("hephos = { path = \"") {
                if let Some(brace) = cargo_content[idx..].find('}') {
                    let end = idx + brace;
                    let before_brace = &cargo_content[..end];
                    let after_brace = &cargo_content[end..];
                    updated_cargo = format!(
                        "{before_brace}, features = [\"db\"]{after_brace}\nsqlx = {{ version = \"0.8\", default-features = false, features = [\"runtime-tokio\", \"tls-rustls\", \"postgres\", \"macros\", \"uuid\", \"chrono\"] }}\nuuid = {{ version = \"1\", features = [\"v4\", \"serde\"] }}\n"
                    );
                }
            }
            if updated_cargo != cargo_content {
                fs::write(&cargo_file, updated_cargo)?;
                created_files.push("Cargo.toml (enabled features = [\"db\"])".to_string());
            }
        }
    }

    // 5. If migrations/ directory exists, create resource migration if absent
    let migrations_dir = project_root.join("migrations");
    if migrations_dir.exists() {
        let migration_pattern = format!("create_{plural}");
        let already_migrated = fs::read_dir(&migrations_dir)
            .ok()
            .map(|entries| {
                entries
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().contains(&migration_pattern))
            })
            .unwrap_or(false);

        if !already_migrated {
            let mig_file = generate_migration(project_root, &format!("create_{plural}"))?;
            created_files.push(mig_file);
        }
    }

    Ok(created_files)
}

/// Generate a canonical SQL migration file in the migrations/ directory.
pub fn generate_migration(project_root: &Path, name: &str) -> Result<String> {
    let clean_name = validate_migration_name(name)?;

    let migrations_dir = project_root.join("migrations");
    fs::create_dir_all(&migrations_dir)?;

    let mut max_seq = 0u64;
    if let Ok(entries) = fs::read_dir(&migrations_dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.ends_with(".sql") {
                if let Some((prefix, _)) = fname.split_once('_') {
                    if let Ok(seq) = prefix.parse::<u64>() {
                        if seq > max_seq {
                            max_seq = seq;
                        }
                    }
                }
            }
        }
    }

    let next_seq = max_seq + 1;
    let filename = format!("{:04}_{clean_name}.sql", next_seq);
    let target_path = migrations_dir.join(&filename);
    if !target_path.starts_with(&migrations_dir) {
        bail!("migration file path escapes migrations directory");
    }

    if target_path.exists() {
        bail!("migration file '{}' already exists", filename);
    }

    let table_name = clean_name
        .trim_start_matches("create_")
        .trim_end_matches("_table");

    let content = format!(
        r#"-- Migration: {filename}
-- Canonical Hephos SQLx migration

CREATE TABLE IF NOT EXISTS {table_name} (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"#
    );

    fs::write(&target_path, content)?;
    Ok(format!("migrations/{filename}"))
}

fn wire_api_routes(content: &str, plural: &str) -> String {
    let merge_call = format!(".merge({plural}::routes())");
    if content.contains(&merge_call) {
        return content.to_string();
    }

    if let Some(fn_idx) = content.find("fn routes") {
        let after_fn = &content[fn_idx..];
        if let Some(router_pos) = after_fn.find("Router::new()") {
            let router_abs = fn_idx + router_pos;
            let rest = &content[router_abs..];
            if let Some(semi_pos) = rest.find(';') {
                let insert_pos = router_abs + semi_pos;
                let mut res = String::new();
                res.push_str(&content[..insert_pos]);
                res.push('\n');
                res.push_str(&format!("        {merge_call}"));
                res.push_str(&content[insert_pos..]);
                return res;
            } else if let Some(brace_pos) = rest.find('}') {
                let insert_pos = router_abs + brace_pos;
                let trimmed_before = content[..insert_pos].trim_end();
                let mut res = String::new();
                res.push_str(trimmed_before);
                res.push('\n');
                res.push_str(&format!("        {merge_call}\n"));
                res.push_str(&content[insert_pos..]);
                return res;
            }
        }
    }
    content.to_string()
}

fn wire_state_service(content: &str, plural: &str, type_name: &str) -> (String, bool) {
    let field_decl = format!("pub {plural}: {type_name}Service,");
    if content.contains(&field_decl) || content.contains(&format!("pub {plural}:")) {
        return (content.to_string(), false);
    }

    let mut result = content.to_string();
    let import_line = format!("use crate::services::{plural}::{type_name}Service;\n");
    if !result.contains(&import_line) && !result.contains(&format!("{type_name}Service")) {
        result = format!("{import_line}{result}");
    }

    let has_db = result.contains("pub db: Db") || result.contains("db: Db");

    // 1. If AppState doesn't have db, add pub db: Db
    if !has_db {
        if let Some(struct_idx) = result.find("pub struct AppState {") {
            let after_struct = &result[struct_idx..];
            if let Some(close_brace) = after_struct.find('}') {
                let insert_pos = struct_idx + close_brace;
                let mut updated = String::new();
                updated.push_str(&result[..insert_pos]);
                updated.push_str("    pub db: Db,\n");
                updated.push_str(&result[insert_pos..]);
                result = updated;
            }
        }
    }

    // 2. Add field to AppState struct
    if let Some(struct_idx) = result.find("pub struct AppState {") {
        let after_struct = &result[struct_idx..];
        if let Some(close_brace) = after_struct.find('}') {
            let insert_pos = struct_idx + close_brace;
            let mut updated = String::new();
            updated.push_str(&result[..insert_pos]);
            updated.push_str(&format!("    {field_decl}\n"));
            updated.push_str(&result[insert_pos..]);
            result = updated;
        }
    }

    // 3. Initialize db in AppState::init() if not present
    if !has_db {
        if let Some(fn_init) = result.find("pub async fn init()") {
            let after_init = &result[fn_init..];
            if let Some(body_start) = after_init.find('{') {
                let insert_pos = fn_init + body_start + 1;
                let db_init = r#"
        let db_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://localhost/postgres".to_string());
        let db = Db::connect_lazy(&db_url)?;
"#;
                let mut updated = String::new();
                updated.push_str(&result[..insert_pos]);
                updated.push_str(db_init);
                updated.push_str(&result[insert_pos..]);
                result = updated;
            }
        }
    }

    // 4. Instantiate service in Ok(Self { ... })
    if let Some(init_idx) = result.find("Ok(Self {") {
        let after_init = &result[init_idx..];
        if let Some(close_brace) = after_init.find('}') {
            let insert_pos = init_idx + close_brace;
            let mut updated = String::new();
            updated.push_str(&result[..insert_pos]);
            if !has_db {
                updated.push_str("            db,\n");
            }
            updated.push_str(&format!(
                "            {plural}: {type_name}Service::new(db.clone()),\n"
            ));
            updated.push_str(&result[insert_pos..]);
            result = updated;
        }
    }

    (result, true)
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize = true;
    for c in s.chars() {
        if c == '_' || c == '-' {
            capitalize = true;
        } else if capitalize {
            result.extend(c.to_uppercase());
            capitalize = false;
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("hephos_test_gen_{}_{}", std::process::id(), id));
            let _ = fs::remove_dir_all(&p);
            let _ = fs::create_dir_all(&p);
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_generate_resource() {
        let temp = TempDir::new();
        let src = temp.path().join("src");
        fs::create_dir_all(&src).unwrap();

        let generated = generate_resource(temp.path(), "product").unwrap();
        assert_eq!(generated.len(), 5);
        assert!(src.join("domain/product.rs").exists());
        assert!(src.join("repos/products.rs").exists());
        assert!(src.join("repos.rs").exists());
        assert!(src.join("services/products.rs").exists());
        assert!(src.join("api/products.rs").exists());

        // Test with pre-existing module files
        let temp2 = TempDir::new();
        let src2 = temp2.path().join("src");
        fs::create_dir_all(&src2).unwrap();
        fs::write(
            src2.join("main.rs"),
            "mod domain;\nmod services;\nmod api;\n",
        )
        .unwrap();
        fs::write(src2.join("domain.rs"), "pub mod other;\n").unwrap();
        fs::write(src2.join("services.rs"), "pub mod other;\n").unwrap();
        fs::write(
            src2.join("api.rs"),
            "pub mod other;\nuse hephos::prelude::*;\nuse crate::state::AppState;\npub fn routes() -> Router<AppState> {\n    Router::new()\n}\n",
        )
        .unwrap();
        fs::write(
            src2.join("state.rs"),
            "use hephos::prelude::*;\npub type Ctx = hephos::Ctx<AppState>;\n#[derive(Clone)]\npub struct AppState {\n    pub db: Db,\n}\nimpl AppState {\n    pub async fn init() -> Result<Self> {\n        let db_url = \"postgres://localhost/postgres\";\n        let db = Db::connect_lazy(db_url)?;\n        Ok(Self {\n            db,\n        })\n    }\n}\n",
        )
        .unwrap();

        let _generated2 = generate_resource(temp2.path(), "order").unwrap();
        assert!(src2.join("domain/order.rs").exists());
        assert!(src2.join("repos/orders.rs").exists());
        assert!(src2.join("repos.rs").exists());
        assert!(
            fs::read_to_string(src2.join("main.rs"))
                .unwrap()
                .contains("mod repos;")
        );
        assert!(
            fs::read_to_string(src2.join("domain.rs"))
                .unwrap()
                .contains("pub mod order;")
        );
        assert!(
            fs::read_to_string(src2.join("services.rs"))
                .unwrap()
                .contains("pub mod orders;")
        );
        let api_content = fs::read_to_string(src2.join("api.rs")).unwrap();
        assert!(api_content.contains("pub mod orders;"));
        assert!(api_content.contains(".merge(orders::routes())"));

        let state_content = fs::read_to_string(src2.join("state.rs")).unwrap();
        assert!(state_content.contains("pub orders: OrderService,"));
        assert!(state_content.contains("OrderService::new(db.clone())"));
    }

    #[test]
    fn test_generate_migration_creates_sequenced_files() {
        let temp = TempDir::new();
        let m1 = generate_migration(temp.path(), "create_users").unwrap();
        assert_eq!(m1, "migrations/0001_create_users.sql");
        assert!(
            temp.path()
                .join("migrations/0001_create_users.sql")
                .exists()
        );

        let m2 = generate_migration(temp.path(), "add_orders_table").unwrap();
        assert_eq!(m2, "migrations/0002_add_orders_table.sql");
        assert!(
            temp.path()
                .join("migrations/0002_add_orders_table.sql")
                .exists()
        );

        let content = fs::read_to_string(temp.path().join(&m1)).unwrap();
        assert!(content.contains("CREATE TABLE IF NOT EXISTS users"));
    }

    #[test]
    fn test_generate_resource_security_validation() {
        let temp = TempDir::new();
        let src = temp.path().join("src");
        fs::create_dir_all(&src).unwrap();

        // 1. Path traversal attempts
        assert!(generate_resource(temp.path(), "../evil").is_err());
        assert!(generate_resource(temp.path(), "../../etc/passwd").is_err());
        assert!(generate_resource(temp.path(), "foo/bar").is_err());
        assert!(generate_resource(temp.path(), "foo\\bar").is_err());
        assert!(generate_resource(temp.path(), "foo\0bar").is_err());

        // 2. Reserved Rust keywords
        assert!(generate_resource(temp.path(), "type").is_err());
        assert!(generate_resource(temp.path(), "struct").is_err());
        assert!(generate_resource(temp.path(), "match").is_err());
        assert!(generate_resource(temp.path(), "fn").is_err());
        assert!(generate_resource(temp.path(), "mod").is_err());
        assert!(generate_resource(temp.path(), "const").is_err());

        // 3. Invalid identifier syntax and code injection attempts
        assert!(generate_resource(temp.path(), "item; rm -rf /").is_err());
        assert!(generate_resource(temp.path(), "item-name").is_err());
        assert!(generate_resource(temp.path(), "123product").is_err());
        assert!(generate_resource(temp.path(), "").is_err());
        assert!(generate_resource(temp.path(), "s").is_err());
    }

    #[test]
    fn test_generate_migration_security_validation() {
        let temp = TempDir::new();

        // 1. Path traversal attempts
        assert!(generate_migration(temp.path(), "../evil").is_err());
        assert!(generate_migration(temp.path(), "sub/evil").is_err());
        assert!(generate_migration(temp.path(), "sub\\evil").is_err());
        assert!(generate_migration(temp.path(), "evil\0name").is_err());

        // 2. SQL injection attempts
        assert!(generate_migration(temp.path(), "create_users; DROP TABLE users; --").is_err());
        assert!(generate_migration(temp.path(), "users' OR '1'='1").is_err());
        assert!(generate_migration(temp.path(), "users/*comment*/").is_err());

        // 3. Valid migration names succeed
        assert!(generate_migration(temp.path(), "create_users").is_ok());
        assert!(generate_migration(temp.path(), "add-indexes").is_ok());
    }
}
