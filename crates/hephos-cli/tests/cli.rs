//! Integration tests for hephos-cli (`hephos new` and `hephos routes`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use hephos_cli::routes::{DiscoveredRoute, format_json, format_table, scan_routes};
use hephos_cli::scaffold::{scaffold_app, validate_package_name};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "hephos_test_{prefix}_{}_{}",
            std::process::id(),
            count
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_validate_package_name() {
    assert!(validate_package_name("my_app").is_ok());
    assert!(validate_package_name("my-service").is_ok());
    assert!(validate_package_name("app123").is_ok());

    assert!(validate_package_name("").is_err());
    assert!(validate_package_name("123app").is_err());
    assert!(validate_package_name("my app").is_err());
    assert!(validate_package_name("my@app").is_err());

    // Reserved keywords
    assert!(validate_package_name("fn").is_err());
    assert!(validate_package_name("struct").is_err());
    assert!(validate_package_name("mod").is_err());
    assert!(validate_package_name("crate").is_err());
}

#[test]
fn test_scaffold_creates_canonical_files() {
    let temp = TempDir::new("scaffold");
    let app_dir = temp.path().join("demo_app");

    scaffold_app("demo_app", &app_dir, None, false).expect("scaffold succeeds");

    assert!(app_dir.join("Cargo.toml").exists());
    assert!(app_dir.join(".gitignore").exists());
    assert!(app_dir.join("src/main.rs").exists());
    assert!(app_dir.join("src/state.rs").exists());
    assert!(app_dir.join("src/api.rs").exists());
    assert!(app_dir.join("src/api/hello.rs").exists());
    assert!(app_dir.join("src/domain.rs").exists());
    assert!(app_dir.join("src/domain/hello.rs").exists());
    assert!(app_dir.join("src/services.rs").exists());
    assert!(app_dir.join("src/services/hello.rs").exists());
    assert!(app_dir.join("src/tests.rs").exists());

    // Verify error on non-empty existing directory
    let err = scaffold_app("demo_app", &app_dir, None, false);
    assert!(err.is_err());
    let err_msg = err.unwrap_err().to_string();
    assert!(err_msg.contains("already exists and is not empty"));
}

#[test]
fn test_scaffold_with_db_creates_database_ready_state_and_deps() {
    let temp = TempDir::new("scaffold_db");
    let app_dir = temp.path().join("db_app");

    scaffold_app("db_app", &app_dir, None, true).expect("scaffold with db succeeds");

    let cargo_toml = fs::read_to_string(app_dir.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("features = [\"db\"]"));
    assert!(cargo_toml.contains("sqlx"));
    assert!(cargo_toml.contains("uuid"));

    let state_rs = fs::read_to_string(app_dir.join("src/state.rs")).unwrap();
    assert!(state_rs.contains("pub db: Db"));
    assert!(state_rs.contains("Db::connect_lazy"));
}

#[test]
#[ignore = "requires crates.io or network access to compile scaffolded crate outside workspace"]
fn test_scaffolded_app_builds_and_passes_tests() {
    let temp = TempDir::new("acceptance");
    let app_dir = temp.path().join("acceptance_app");

    // Local path to crates/hephos for offline compilation in test
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let local_hephos = repo_root.join("crates").join("hephos");

    scaffold_app("acceptance_app", &app_dir, Some(&local_hephos), false)
        .expect("scaffold succeeds");

    // Run `cargo test` in the generated app directory
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .arg("test")
        .current_dir(&app_dir)
        .status()
        .expect("cargo test execution");

    assert!(status.success(), "generated app failed cargo test");
}

#[test]
fn test_scan_routes_from_real_example() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let rest_dir = repo_root.join("examples").join("rest");

    let routes = scan_routes(&rest_dir).expect("scan_routes succeeds on example-rest");

    // Probes should always be present
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/health" && r.method == "GET")
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/ready" && r.method == "GET")
    );

    // Routes defined in examples/rest/src/api/products.rs
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/products" && r.method == "GET" && r.handler == "list")
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/products/{id}" && r.method == "GET" && r.handler == "get")
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/products/search" && r.method == "POST" && r.handler == "search")
    );

    // Format table
    let table = format_table(&routes);
    assert!(table.contains("METHOD"));
    assert!(table.contains("/products"));
    assert!(table.contains("/health"));

    // Format JSON
    let json_str = format_json(&routes).expect("format_json");
    let deserialized: Vec<DiscoveredRoute> =
        serde_json::from_str(&json_str).expect("deserialize JSON");
    assert_eq!(deserialized, routes);
}

#[test]
fn test_scan_routes_invalid_dir() {
    let temp = TempDir::new("empty");
    let err = scan_routes(temp.path());
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("no 'src/api.rs' or 'src/api' directory found"));
}

#[test]
fn test_openapi_generation_from_real_example() {
    use hephos_cli::routes::generate_openapi;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let rest_dir = repo_root.join("examples").join("rest");

    let routes = scan_routes(&rest_dir).expect("scan_routes succeeds on example-rest");
    let openapi = generate_openapi(&routes, "Products API", "1.0.0");

    assert_eq!(openapi["openapi"], "3.1.0");
    assert_eq!(openapi["info"]["title"], "Products API");
    assert_eq!(openapi["info"]["version"], "1.0.0");
    assert!(openapi["paths"]["/products"]["get"].is_object());
    assert!(openapi["paths"]["/products/search"]["post"].is_object());
    assert!(openapi["paths"]["/health"]["get"].is_object());
    assert!(openapi["paths"]["/ready"]["get"].is_object());
}

#[test]
fn test_end_to_end_scaffold_and_resource_generation() {
    let temp = TempDir::new("fullstack");
    let app_dir = temp.path().join("blog_app");

    // 1. Scaffold database-ready application
    scaffold_app("blog_app", &app_dir, None, true).expect("scaffold with db succeeds");

    // 2. Generate a new resource "article"
    let created = hephos_cli::generate_resource(&app_dir, "article").expect("generate resource");
    assert!(!created.is_empty());

    // 3. Verify files and wiring
    assert!(app_dir.join("src/domain/article.rs").exists());
    assert!(app_dir.join("src/repos/articles.rs").exists());
    assert!(app_dir.join("src/services/articles.rs").exists());
    assert!(app_dir.join("src/api/articles.rs").exists());

    let state_rs = fs::read_to_string(app_dir.join("src/state.rs")).unwrap();
    assert!(state_rs.contains("pub articles: ArticleService,"));
    assert!(state_rs.contains("ArticleService::new(db.clone())"));

    let api_rs = fs::read_to_string(app_dir.join("src/api.rs")).unwrap();
    assert!(api_rs.contains("pub mod articles;"));
    assert!(api_rs.contains(".merge(articles::routes())"));

    // 4. Scan routes on the generated application
    let routes = scan_routes(&app_dir).expect("scan routes on generated app");
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/articles/{id}" && r.method == "GET")
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/articles" && r.method == "POST")
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/hello" && r.method == "GET")
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/health" && r.method == "GET")
    );

    // 5. Generate OpenAPI for the generated application
    let doc = hephos_cli::generate_openapi(&routes, "Blog API", "1.0.0");
    assert!(doc["paths"]["/articles/{id}"]["get"].is_object());
    assert!(doc["paths"]["/articles"]["post"].is_object());
}
