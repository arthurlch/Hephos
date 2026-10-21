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
    let mut in_block_comment = false;
    for (line_idx, line) in content.lines().enumerate() {
        let code_part = strip_comments_line(line, &mut in_block_comment);
        if code_part.trim().is_empty() {
            continue;
        }
        for &(rule, patterns) in rules {
            for &pat in patterns {
                if code_part.contains(pat) {
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

fn strip_comments_line(line: &str, in_block_comment: &mut bool) -> String {
    let mut result = String::new();
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if *in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            *in_block_comment = true;
        } else if c == '/' && chars.peek() == Some(&'/') {
            // Rest of the line is a single-line comment
            break;
        } else {
            result.push(c);
        }
    }

    result
}

fn find_rs_files(dir: &Path, single_file: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if single_file.is_file() {
        files.push(single_file.to_path_buf());
    }
    if dir.is_dir() {
        collect_rs_files(dir, &mut files);
    }
    files.sort();
    files
}

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
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

    #[test]
    fn test_check_project_nested_subdirectories() {
        let temp = TempDir::new();
        let nested_api = temp.path().join("src").join("api").join("v1");
        fs::create_dir_all(&nested_api).unwrap();

        fs::write(nested_api.join("orders.rs"), "use crate::repos::orders;\n").unwrap();

        let violations = check_project(temp.path()).unwrap();
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule,
            "handler imports a repo (api/ must not depend on repos/)"
        );
    }

    #[test]
    fn test_check_project_comment_false_positives() {
        let temp = TempDir::new();
        let api = temp.path().join("src").join("api");
        fs::create_dir_all(&api).unwrap();

        fs::write(
            api.join("docs.rs"),
            r#"
            // Line comment: use crate::repos::items;
            /* Block comment single line: ctx.state().db.pool(); */
            /*
             * Multi-line block comment:
             * use crate::repos;
             * sqlx::query!
             */
            pub async fn handler(ctx: Ctx) -> Result<Json<()>> {
                // Inline comment at end of valid line:
                Ok(Json(())) // see crate::repos for documentation
            }
            "#,
        )
        .unwrap();

        let violations = check_project(temp.path()).unwrap();
        assert!(
            violations.is_empty(),
            "Comments should not trigger violations, got: {violations:?}"
        );
    }
}
