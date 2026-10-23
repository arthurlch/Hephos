//! Route discovery and display logic for `hephos routes`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// A route discovered by scanning a Hephos project's API source files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredRoute {
    /// HTTP method (GET, POST, PUT, PATCH, DELETE).
    pub method: String,
    /// Path template (e.g. `/hello`, `/users/{id}`).
    pub path: String,
    /// Handler function name or description.
    pub handler: String,
    /// Relative path to source file where the route was defined.
    pub source_file: String,
}

/// Scans a Hephos application source tree and returns discovered routes.
pub fn scan_routes(project_root: &Path) -> Result<Vec<DiscoveredRoute>> {
    let src_dir = project_root.join("src");
    let api_file = src_dir.join("api.rs");
    let api_dir = src_dir.join("api");

    if !api_file.exists() && !api_dir.exists() {
        bail!(
            "no 'src/api.rs' or 'src/api' directory found in '{}'. Ensure this command is run from a Hephos project root.",
            project_root.display()
        );
    }

    let mut routes = Vec::new();

    // The framework always auto-provisions canonical /health and /ready probes
    routes.push(DiscoveredRoute {
        method: "GET".to_string(),
        path: "/health".to_string(),
        handler: "probe (liveness)".to_string(),
        source_file: "(framework)".to_string(),
    });
    routes.push(DiscoveredRoute {
        method: "GET".to_string(),
        path: "/ready".to_string(),
        handler: "probe (readiness)".to_string(),
        source_file: "(framework)".to_string(),
    });

    let mut candidate_files = Vec::new();
    if api_file.exists() {
        candidate_files.push(api_file);
    }
    if api_dir.exists() {
        collect_rs_files(&api_dir, &mut candidate_files)?;
    }

    for file_path in candidate_files {
        let content = fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read {}", file_path.display()))?;
        let relative_source = file_path
            .strip_prefix(project_root)
            .unwrap_or(&file_path)
            .display()
            .to_string()
            .replace('\\', "/");

        let extracted = parse_routes_from_str(&content, &relative_source);
        routes.extend(extracted);
    }

    // Sort: probes first, then by path, then by method
    routes.sort_by(|a, b| {
        let a_is_probe = a.source_file == "(framework)";
        let b_is_probe = b.source_file == "(framework)";
        match (a_is_probe, b_is_probe) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path.cmp(&b.path).then_with(|| a.method.cmp(&b.method)),
        }
    });

    Ok(routes)
}

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, files)?;
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    Ok(())
}

/// Parses route registrations from Rust source code.
/// Matches both shorthand patterns like `.get("/path", handler)` and Axum patterns like `.route("/path", get(h1).post(h2))`.
pub fn parse_routes_from_str(source: &str, relative_source: &str) -> Vec<DiscoveredRoute> {
    let mut routes = Vec::new();
    let verbs = ["get", "post", "put", "patch", "delete"];

    // 1. Shorthand pattern: .get("/path", handler), .post("/path", handler), etc.
    for verb in verbs {
        let pattern = format!(".{verb}(");
        let mut search_from = 0;

        while let Some(pos) = source[search_from..].find(&pattern) {
            let abs_pos = search_from + pos + pattern.len();
            search_from = abs_pos;

            let remainder = &source[abs_pos..];
            if let Some((path, handler)) = extract_path_and_handler(remainder) {
                routes.push(DiscoveredRoute {
                    method: verb.to_ascii_uppercase(),
                    path,
                    handler,
                    source_file: relative_source.to_string(),
                });
            }
        }
    }

    // 2. Axum standard pattern: .route("/path", get(h1).post(h2))
    let route_pattern = ".route(";
    let mut search_from = 0;
    while let Some(pos) = source[search_from..].find(route_pattern) {
        let abs_pos = search_from + pos + route_pattern.len();
        search_from = abs_pos;

        let remainder = &source[abs_pos..];
        if let Some(first_quote) = remainder.find('"') {
            let after_quote = &remainder[first_quote + 1..];
            if let Some(second_quote) = after_quote.find('"') {
                let path = after_quote[..second_quote].to_string();
                let after_path = &after_quote[second_quote + 1..];
                if let Some(comma_pos) = after_path.find(',') {
                    let after_comma = &after_path[comma_pos + 1..];
                    let block_end = after_comma.find(';').unwrap_or_else(|| {
                        after_comma.find(".route(").unwrap_or(after_comma.len())
                    });
                    let route_body = &after_comma[..block_end];

                    for verb in verbs {
                        let v_patterns = [format!("{verb}("), format!(".{verb}(")];
                        for vp in v_patterns {
                            let mut v_search = 0;
                            while let Some(v_pos) = route_body[v_search..].find(&vp) {
                                let h_start = v_search + v_pos + vp.len();
                                v_search = h_start;
                                let h_rem = &route_body[h_start..];
                                if let Some(h_end) = h_rem.find(')') {
                                    let handler = h_rem[..h_end].trim().to_string();
                                    if !handler.is_empty() {
                                        routes.push(DiscoveredRoute {
                                            method: verb.to_ascii_uppercase(),
                                            path: path.clone(),
                                            handler,
                                            source_file: relative_source.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Deduplicate identical (method, path) entries
    routes.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.method.cmp(&b.method)));
    routes.dedup_by(|a, b| a.method == b.method && a.path == b.path);

    routes
}

fn extract_path_and_handler(call_args: &str) -> Option<(String, String)> {
    // 1. Locate first double quote
    let first_quote = call_args.find('"')?;
    let after_quote = &call_args[first_quote + 1..];
    let second_quote = after_quote.find('"')?;
    let path = after_quote[..second_quote].to_string();

    // 2. Locate comma separating path from handler
    let after_path = &after_quote[second_quote + 1..];
    let comma = after_path.find(',')?;
    let after_comma = &after_path[comma + 1..];

    // 3. Extract handler identifier up to next ')', ',', or whitespace
    let trimmed = after_comma.trim_start();
    let handler_end = trimmed
        .find(|c: char| c == ')' || c == ',' || c.is_whitespace())
        .unwrap_or(trimmed.len());

    let handler = trimmed[..handler_end].trim().to_string();
    if handler.is_empty() {
        return None;
    }

    Some((path, handler))
}

/// Formats a list of discovered routes as an aligned ASCII table.
pub fn format_table(routes: &[DiscoveredRoute]) -> String {
    if routes.is_empty() {
        return "No routes discovered.\n".to_string();
    }

    let method_width = routes
        .iter()
        .map(|r| r.method.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let path_width = routes
        .iter()
        .map(|r| r.path.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let handler_width = routes
        .iter()
        .map(|r| r.handler.len())
        .max()
        .unwrap_or(7)
        .max(7);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<m_w$}  {:<p_w$}  {:<h_w$}  SOURCE\n",
        "METHOD",
        "PATH",
        "HANDLER",
        m_w = method_width,
        p_w = path_width,
        h_w = handler_width,
    ));
    out.push_str(&format!(
        "{:<m_w$}  {:<p_w$}  {:<h_w$}  ------\n",
        "------",
        "----",
        "-------",
        m_w = method_width,
        p_w = path_width,
        h_w = handler_width,
    ));

    for route in routes {
        out.push_str(&format!(
            "{:<m_w$}  {:<p_w$}  {:<h_w$}  {}\n",
            route.method,
            route.path,
            route.handler,
            route.source_file,
            m_w = method_width,
            p_w = path_width,
            h_w = handler_width,
        ));
    }

    out
}

/// Formats a list of discovered routes as a pretty-printed JSON string.
pub fn format_json(routes: &[DiscoveredRoute]) -> Result<String> {
    serde_json::to_string_pretty(routes).context("failed to serialize route table to JSON")
}

/// Generate an OpenAPI 3.1.0 document from discovered routes.
pub fn generate_openapi(
    routes: &[DiscoveredRoute],
    title: &str,
    version: &str,
) -> serde_json::Value {
    let mut paths = serde_json::Map::new();

    for route in routes {
        let path_item = paths
            .entry(&route.path)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

        if let serde_json::Value::Object(methods) = path_item {
            let method_key = route.method.to_lowercase();

            // Extract path parameters if any: e.g. /users/{id} -> parameter "id"
            let mut parameters = Vec::new();
            let mut start = 0;
            while let Some(open) = route.path[start..].find('{') {
                let open_abs = start + open + 1;
                if let Some(close) = route.path[open_abs..].find('}') {
                    let param_name = &route.path[open_abs..open_abs + close];
                    parameters.push(serde_json::json!({
                        "name": param_name,
                        "in": "path",
                        "required": true,
                        "schema": {
                            "type": "string"
                        }
                    }));
                    start = open_abs + close + 1;
                } else {
                    break;
                }
            }

            let mut op = serde_json::json!({
                "summary": format!("{} {}", route.method, route.path),
                "operationId": format!("{}_{}", method_key, route.handler),
                "responses": {
                    "200": { "description": "Successful response" },
                    "default": {
                        "description": "Error response conforming to Hephos canonical error envelope",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "error": {
                                            "type": "object",
                                            "properties": {
                                                "kind": { "type": "string" },
                                                "message": { "type": "string" }
                                            },
                                            "required": ["kind", "message"]
                                        }
                                    },
                                    "required": ["error"]
                                }
                            }
                        }
                    }
                }
            });

            if !parameters.is_empty() {
                op["parameters"] = serde_json::Value::Array(parameters);
            }

            methods.insert(method_key, op);
        }
    }

    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": title,
            "version": version
        },
        "paths": paths
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_openapi_doc() {
        let routes = vec![
            DiscoveredRoute {
                method: "GET".to_string(),
                path: "/users".to_string(),
                handler: "list_users".to_string(),
                source_file: "src/api/users.rs".to_string(),
            },
            DiscoveredRoute {
                method: "POST".to_string(),
                path: "/users".to_string(),
                handler: "create_user".to_string(),
                source_file: "src/api/users.rs".to_string(),
            },
        ];

        let doc = generate_openapi(&routes, "Test API", "1.0.0");
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["title"], "Test API");
        assert_eq!(doc["info"]["version"], "1.0.0");
        assert!(doc["paths"]["/users"]["get"].is_object());
        assert!(doc["paths"]["/users"]["post"].is_object());
    }

    #[test]
    fn parse_routes_discovers_both_shorthand_and_axum_route_syntax() {
        let source = r#"
            pub fn routes() -> Router<AppState> {
                Router::new()
                    .route("/users", get(list_users).post(create_user))
                    .route("/users/{id}", get(get_user).delete(delete_user))
                    .get("/health", health_check)
            }
        "#;
        let routes = parse_routes_from_str(source, "src/api.rs");

        assert!(
            routes
                .iter()
                .any(|r| r.path == "/users" && r.method == "GET" && r.handler == "list_users")
        );
        assert!(
            routes
                .iter()
                .any(|r| r.path == "/users" && r.method == "POST" && r.handler == "create_user")
        );
        assert!(
            routes
                .iter()
                .any(|r| r.path == "/users/{id}" && r.method == "GET" && r.handler == "get_user")
        );
        assert!(routes.iter().any(|r| r.path == "/users/{id}"
            && r.method == "DELETE"
            && r.handler == "delete_user"));
        assert!(
            routes
                .iter()
                .any(|r| r.path == "/health" && r.method == "GET" && r.handler == "health_check")
        );
    }

    #[test]
    fn openapi_includes_path_parameters_and_canonical_error_schema() {
        let routes = vec![DiscoveredRoute {
            method: "GET".to_string(),
            path: "/users/{user_id}/orders/{order_id}".to_string(),
            handler: "get_user_order".to_string(),
            source_file: "src/api/users.rs".to_string(),
        }];

        let doc = generate_openapi(&routes, "Orders API", "1.0.0");
        let op = &doc["paths"]["/users/{user_id}/orders/{order_id}"]["get"];
        let params = op["parameters"].as_array().expect("parameters array");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0]["name"], "user_id");
        assert_eq!(params[0]["in"], "path");
        assert_eq!(params[0]["required"], true);
        assert_eq!(params[1]["name"], "order_id");

        // Canonical error envelope schema on default response
        let default_resp = &op["responses"]["default"];
        assert!(
            default_resp["content"]["application/json"]["schema"]["properties"]["error"]
                .is_object()
        );
    }
}
