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
        if !content.contains(&line) {
            let mut new_c = content;
            if !new_c.ends_with('\n') {
                new_c.push('\n');
            }
            new_c.push_str(&line);
            new_c.push('\n');
            fs::write(&api_mod, new_c)?;
            created_files.push(format!("src/api.rs (declared pub mod {plural})"));
        }
    }

    Ok(created_files)
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
        fs::write(src2.join("api.rs"), "pub mod other;\n").unwrap();

        let generated2 = generate_resource(temp2.path(), "order").unwrap();
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
        assert!(
            fs::read_to_string(src2.join("api.rs"))
                .unwrap()
                .contains("pub mod orders;")
        );
        assert_eq!(generated2.len(), 9); // 4 resource files + repos.rs + 4 wiring updates
    }
}
