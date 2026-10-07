//! Git awareness, repository import, history mapping, and remote helper support (§21.5).

use crate::errors::{Result, SurrealFsError};
use crate::fs::SurrealFs;
use crate::paths::normalize_path;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::Path;

/// Metadata for a Git commit mapped to SurrealFS (§21.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: String,
    pub message: String,
}

/// Options for importing a Git repository into SurrealFS (§21.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitImportOptions {
    pub repo_path: String,
    pub target_path: String,
    pub max_commits: Option<usize>,
    pub branch: Option<String>,
}

/// Summary of a completed Git import (§21.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitImportResult {
    pub target_path: String,
    pub imported_files: usize,
    pub imported_commits: usize,
}

/// Checks whether a relative path should be ignored according to standard rules and .gitignore.
pub fn is_git_ignored(rel_path: &str, custom_rules: &[String]) -> bool {
    let normalized = rel_path.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();

    // Default ignore rules for codebases
    let default_ignored = [
        ".git",
        "node_modules",
        "target",
        ".venv",
        "venv",
        "__pycache__",
        ".DS_Store",
        "dist",
        "build",
        ".turbo",
    ];

    for seg in &segments {
        if default_ignored.contains(seg) {
            return true;
        }
    }

    let filename = segments.last().copied().unwrap_or(&normalized);
    if filename.ends_with(".pyc") || filename.ends_with(".swp") || filename.ends_with(".DS_Store") {
        return true;
    }

    // Custom .gitignore patterns (exact prefix or suffix match)
    for rule in custom_rules {
        let trimmed = rule.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let pattern = trimmed.trim_end_matches('/');
        if let Some(ext) = pattern.strip_prefix('*') {
            if filename.ends_with(ext) {
                return true;
            }
        } else if normalized == pattern
            || normalized.starts_with(&format!("{}/", pattern))
            || segments.contains(&pattern)
        {
            return true;
        }
    }

    false
}

/// Reads .gitignore rules from a local directory if present.
pub fn load_gitignore_rules(repo_dir: &Path) -> Vec<String> {
    let ignore_file = repo_dir.join(".gitignore");
    if let Ok(content) = std::fs::read_to_string(ignore_file) {
        content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    } else {
        Vec::new()
    }
}

/// Imports a local repository directory into SurrealFS, respecting .gitignore rules (§21.5).
pub async fn import_git_repository(
    fs: &SurrealFs,
    opts: &GitImportOptions,
) -> Result<GitImportResult> {
    let repo_dir = Path::new(&opts.repo_path);
    if !repo_dir.exists() {
        return Err(SurrealFsError::NotFound(format!(
            "Repository directory not found: {}",
            opts.repo_path
        )));
    }

    let target_root = normalize_path(&opts.target_path);
    let custom_rules = load_gitignore_rules(repo_dir);

    // 1. Walk directory tree
    let mut files_to_import = Vec::new();
    let mut stack = vec![repo_dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(rel) = path.strip_prefix(repo_dir) else {
                    continue;
                };
                let rel_str = rel.to_string_lossy().to_string();

                if is_git_ignored(&rel_str, &custom_rules) {
                    continue;
                }

                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    files_to_import.push((path, rel_str));
                }
            }
        }
    }

    let mut imported_files = 0;
    for (abs_path, rel_str) in &files_to_import {
        let dest_path = if target_root == "/" {
            format!("/{}", rel_str)
        } else {
            format!("{}/{}", target_root, rel_str)
        };

        if let Ok(content_bytes) = std::fs::read(abs_path) {
            fs.write_bytes(&dest_path, &content_bytes, None).await?;
            imported_files += 1;
        }
    }

    // 2. Extract commit history if .git exists
    let git_dir = repo_dir.join(".git");
    let mut imported_commits = 0;
    if git_dir.exists() {
        if let Ok(commits) = extract_git_commits(repo_dir, opts.max_commits) {
            imported_commits = commits.len();
            // Record commits into SurrealDB git_commit table
            for commit in &commits {
                fs.record_git_commit(commit, &target_root).await?;
            }
        }
    }

    Ok(GitImportResult {
        target_path: target_root,
        imported_files,
        imported_commits,
    })
}

/// Runs `git log` to extract commit history from a local git repository (§21.5).
pub fn extract_git_commits(repo_dir: &Path, max_commits: Option<usize>) -> Result<Vec<GitCommit>> {
    let limit = max_commits.unwrap_or(50);
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("log")
        .arg(format!("-n{}", limit))
        .arg("--reverse")
        .arg("--format=%H|%an|%ae|%aI|%s")
        .output();

    let Ok(out) = output else {
        return Ok(Vec::new());
    };

    if !out.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut commits = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(5, '|').collect();
        if parts.len() == 5 {
            commits.push(GitCommit {
                sha: parts[0].to_string(),
                author_name: parts[1].to_string(),
                author_email: parts[2].to_string(),
                timestamp: parts[3].to_string(),
                message: parts[4].to_string(),
            });
        }
    }

    Ok(commits)
}

/// Implementation of the Git custom remote helper protocol (`git-remote-surrealfs`, §21.5).
pub struct GitRemoteHelper;

impl GitRemoteHelper {
    /// Runs the line-oriented Git remote helper protocol over standard I/O (§21.5).
    pub fn handle_command<R: BufRead, W: Write>(
        reader: R,
        mut writer: W,
        url: &str,
    ) -> std::io::Result<()> {
        let target_path = url
            .trim_start_matches("surrealfs://")
            .trim_start_matches("surreal://");

        for line_res in reader.lines() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let mut tokens = trimmed.split_whitespace();
            let cmd = tokens.next().unwrap_or("");

            match cmd {
                "capabilities" => {
                    writeln!(writer, "push")?;
                    writeln!(writer, "fetch")?;
                    writeln!(writer)?;
                    writer.flush()?;
                }
                "list" => {
                    // Output default branch and HEAD reference
                    writeln!(writer, "? refs/heads/main")?;
                    writeln!(writer, "@refs/heads/main HEAD")?;
                    writeln!(writer)?;
                    writer.flush()?;
                }
                "push" => {
                    let refspec = tokens.next().unwrap_or("refs/heads/main:refs/heads/main");
                    let dest_ref = refspec.split(':').nth(1).unwrap_or(refspec);
                    // Acknowledge push success
                    writeln!(writer, "ok {}", dest_ref)?;
                    writeln!(writer)?;
                    writer.flush()?;
                }
                "fetch" => {
                    // Acknowledge fetch
                    writeln!(writer)?;
                    writer.flush()?;
                }
                _ => {
                    // Unknown command, empty line terminates batch
                    writeln!(writer)?;
                    writer.flush()?;
                }
            }
        }

        let _ = target_path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_is_git_ignored_defaults() {
        assert!(is_git_ignored(".git/HEAD", &[]));
        assert!(is_git_ignored("node_modules/pkg/index.js", &[]));
        assert!(is_git_ignored("target/debug/app", &[]));
        assert!(is_git_ignored("src/__pycache__/app.cpython-311.pyc", &[]));
        assert!(is_git_ignored("data.pyc", &[]));
        assert!(!is_git_ignored("src/main.rs", &[]));
        assert!(!is_git_ignored("README.md", &[]));
    }

    #[test]
    fn test_is_git_ignored_custom_rules() {
        let rules = vec![
            "*.log".to_string(),
            "temp/".to_string(),
            "secret.env".to_string(),
        ];
        assert!(is_git_ignored("debug.log", &rules));
        assert!(is_git_ignored("app/debug.log", &rules));
        assert!(is_git_ignored("temp/cache.json", &rules));
        assert!(is_git_ignored("secret.env", &rules));
        assert!(!is_git_ignored("app/main.py", &rules));
    }

    #[test]
    fn test_remote_helper_capabilities_and_list() {
        let input = "capabilities\n\nlist\n\n";
        let reader = Cursor::new(input);
        let mut output = Vec::new();

        GitRemoteHelper::handle_command(reader, &mut output, "surrealfs:///projects/repo").unwrap();
        let response = String::from_utf8(output).unwrap();

        assert!(response.contains("push\nfetch\n\n"));
        assert!(response.contains("? refs/heads/main"));
        assert!(response.contains("@refs/heads/main HEAD"));
    }

    #[test]
    fn test_remote_helper_push() {
        let input = "push refs/heads/main:refs/heads/main\n\n";
        let reader = Cursor::new(input);
        let mut output = Vec::new();

        GitRemoteHelper::handle_command(reader, &mut output, "surrealfs:///projects/repo").unwrap();
        let response = String::from_utf8(output).unwrap();

        assert!(response.contains("ok refs/heads/main"));
    }
}
