use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Architecture violation record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchViolation {
    pub rule: &'static str,
    pub file: PathBuf,
    pub line_number: usize,
    pub line_content: String,
}

/// Run AGENTS.md §2 architectural boundary checks on a given project directory.
pub fn check_project(root: &Path) -> Result<Vec<ArchViolation>> {
    let src_dir = root.join("src");
    if !src_dir.exists() {
        bail!(
            "directory '{}' does not contain a 'src' directory",
            root.display()
        );
    }

    let mut violations = Vec::new();

    // 1. Handlers (api/) must not run SQL or touch pool/tx
    let api_files = find_rs_files(&src_dir.join("api"), &src_dir.join("api.rs"));
    for file in &api_files {
        scan_file(
            file,
            &mut violations,
            &[
                (
                    "handler runs SQL or touches pool/tx (api/ must call a service)",
                    &["sqlx::", "query!", "query_as!", ".pool(", ".begin("],
                ),
                (
                    "handler imports a repo (api/ must not depend on repos/)",
                    &["use crate::repos"],
                ),
            ],
        )?;
    }

    // 2. Services (services/) must not speak HTTP (axum / StatusCode / IntoResponse)
    let service_files = find_rs_files(&src_dir.join("services"), &src_dir.join("services.rs"));
    for file in &service_files {
        scan_file(
            file,
            &mut violations,
            &[(
                "service speaks HTTP (services/ must be transport-agnostic)",
                &["axum::", "StatusCode", "IntoResponse"],
            )],
        )?;
    }

    // 3. Repos (repos/) must not call up into services
    let repo_files = find_rs_files(&src_dir.join("repos"), &src_dir.join("repos.rs"));
    for file in &repo_files {
        scan_file(
            file,
            &mut violations,
            &[(
                "repo calls up into services (repos/ runs SQL only)",
                &["use crate::services"],
            )],
        )?;
    }

    Ok(violations)
}

fn scan_file(
    file: &Path,
    violations: &mut Vec<ArchViolation>,
    rules: &[(&'static str, &[&'static str])],
) -> Result<()> {
    let content = fs::read_to_string(file)?;
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        for &(rule, patterns) in rules {
            for &pat in patterns {
                if line.contains(pat) {
                    violations.push(ArchViolation {
                        rule,
                        file: file.to_path_buf(),
                        line_number: line_idx + 1,
                        line_content: line.to_string(),
                    });
                    break;
                }
            }
        }
    }
    Ok(())
}

fn find_rs_files(dir: &Path, single_file: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if single_file.is_file() {
        files.push(single_file.to_path_buf());
    }
    if dir.is_dir() {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "hephos_test_check_{}_{}",
                std::process::id(),
                id
            ));
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
    fn test_check_project_clean() {
        let temp = TempDir::new();
        let src = temp.path().join("src");
        let api = src.join("api");
        let services = src.join("services");
        fs::create_dir_all(&api).unwrap();
        fs::create_dir_all(&services).unwrap();

        fs::write(
            api.join("items.rs"),
            "pub async fn list(ctx: Ctx) -> Result<Json<Vec<Item>>> { Ok(Json(vec![])) }\n",
        )
        .unwrap();

        let violations = check_project(temp.path()).unwrap();
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_project_detects_violations() {
        let temp = TempDir::new();
        let src = temp.path().join("src");
        let api = src.join("api");
        fs::create_dir_all(&api).unwrap();

        fs::write(
            api.join("items.rs"),
            "use crate::repos::items;\npub async fn list(ctx: Ctx) { ctx.state().db.pool(); }\n",
        )
        .unwrap();

        let violations = check_project(temp.path()).unwrap();
        assert_eq!(violations.len(), 2);
    }
}
