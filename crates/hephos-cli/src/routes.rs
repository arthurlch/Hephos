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
/// Matches patterns like `.get("/path", handler)` across lines.
pub fn parse_routes_from_str(source: &str, relative_source: &str) -> Vec<DiscoveredRoute> {
    let mut routes = Vec::new();
    let verbs = ["get", "post", "put", "patch", "delete"];

    for verb in verbs {
        let pattern = format!(".{verb}(");
        let mut search_from = 0;

        while let Some(pos) = source[search_from..].find(&pattern) {
            let abs_pos = search_from + pos + pattern.len();
            search_from = abs_pos;

            // Find closing paren of the call
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
