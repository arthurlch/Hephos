use std::fs;
use std::path::Path;

use anyhow::{Result, bail};

/// Generate canonical domain, repo, service, and api scaffold files for a resource.
pub fn generate_resource(project_root: &Path, resource: &str) -> Result<Vec<String>> {
    let raw_name = resource.trim().to_lowercase();
    if raw_name.is_empty() {
        bail!("resource name cannot be empty");
    }

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

    Ok(created_files)
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
}
