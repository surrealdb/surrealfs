use std::time::{SystemTime, UNIX_EPOCH};

use globset::{GlobBuilder, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use surrealdb::{Surreal, engine::remote::ws::Client};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, FsError>;

pub mod curl;

#[cfg(feature = "python")]
pub mod python;

#[derive(Debug, Error)]
pub enum FsError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("not a file: {0}")]
    NotAFile(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("invalid path")]
    InvalidPath,
    #[error("http error: {0}")]
    Http(String),
    #[error("database error: {0}")]
    Surreal(#[from] surrealdb::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub parent: Option<String>,
    pub is_dir: bool,
    pub content: Option<String>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NumberedLine {
    pub number: usize,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

/// SurrealDB-backed filesystem facade. The client connection is provided by the caller.
pub struct SurrealFs<DB = Client>
where
    DB: surrealdb::Connection,
{
    db: Surreal<DB>,
    table: String,
}

impl<DB> SurrealFs<DB>
where
    DB: surrealdb::Connection,
{
    pub fn new(db: Surreal<DB>) -> Self {
        Self {
            db,
            table: "fs_entry".into(),
        }
    }

    pub fn with_table(db: Surreal<DB>, table: impl Into<String>) -> Self {
        Self {
            db,
            table: table.into(),
        }
    }

    pub async fn ls(&self, path: impl AsRef<str>) -> Result<Vec<Entry>> {
        let path = normalize_path(path.as_ref())?;
        if path == "/" {
            return self.children(&path).await;
        }

        if let Some(entry) = self.get_entry(&path).await? {
            if entry.is_dir {
                self.children(&path).await
            } else {
                Ok(vec![entry])
            }
        } else {
            Err(FsError::NotFound(path))
        }
    }

    pub async fn cat(&self, path: impl AsRef<str>) -> Result<String> {
        let entry = self.require_file(path.as_ref()).await?;
        Ok(entry.content.unwrap_or_default())
    }

    pub async fn tail(&self, path: impl AsRef<str>, n: usize) -> Result<Vec<String>> {
        let content = self.cat(path.as_ref()).await?;
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(n);
        Ok(lines[start..].iter().map(|s| s.to_string()).collect())
    }

    pub async fn read(
        &self,
        path: impl AsRef<str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let content = self.cat(path.as_ref()).await?;
        let lines: Vec<&str> = content.lines().collect();
        let start = offset.min(lines.len());
        let end = start.saturating_add(limit).min(lines.len());
        Ok(lines[start..end].iter().map(|s| s.to_string()).collect())
    }

    pub async fn nl(&self, path: impl AsRef<str>, start_at: usize) -> Result<Vec<NumberedLine>> {
        let content = self.cat(path.as_ref()).await?;
        Ok(content
            .lines()
            .enumerate()
            .map(|(idx, line)| NumberedLine {
                number: start_at + idx,
                line: line.to_string(),
            })
            .collect())
    }

    pub async fn grep(
        &self,
        pattern: &Regex,
        path: impl AsRef<str>,
        recursive: bool,
    ) -> Result<Vec<GrepMatch>> {
        let path = normalize_path(path.as_ref())?;
        let mut matches = Vec::new();
        let mut stack = vec![path.clone()];
        while let Some(p) = stack.pop() {
            let entry = match self.get_entry(&p).await? {
                Some(e) => e,
                None => return Err(FsError::NotFound(p)),
            };
            if entry.is_dir {
                if recursive {
                    for child in self.children(&p).await? {
                        stack.push(child.path);
                    }
                }
            } else if let Some(content) = &entry.content {
                for (idx, line) in content.lines().enumerate() {
                    if pattern.is_match(line) {
                        matches.push(GrepMatch {
                            path: entry.path.clone(),
                            line_number: idx + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
        }
        Ok(matches)
    }

    pub async fn glob(&self, pattern: impl AsRef<str>) -> Result<Vec<String>> {
        let pattern = pattern.as_ref();
        if pattern.is_empty() {
            return Err(FsError::InvalidPath);
        }

        let normalized = normalize_path(pattern)?;
        let trimmed = normalized.trim_start_matches('/');
        if trimmed.is_empty() {
            return Err(FsError::InvalidPath);
        }

        let mut builder = GlobSetBuilder::new();
        let trimmed_glob = GlobBuilder::new(trimmed)
            .literal_separator(true)
            .build()
            .map_err(|_| FsError::InvalidPath)?;
        builder.add(trimmed_glob);

        if trimmed != normalized {
            let absolute_glob = GlobBuilder::new(&normalized)
                .literal_separator(true)
                .build()
                .map_err(|_| FsError::InvalidPath)?;
            builder.add(absolute_glob);
        }

        let matcher = builder.build().map_err(|_| FsError::InvalidPath)?;

        let mut res = self
            .db
            .query(format!(
                "SELECT path, name, parent, is_dir, content, updated_at FROM {}",
                self.table
            ))
            .await?;
        let mut entries: Vec<Entry> = res.take(0)?;

        entries.retain(|entry| {
            let path = entry.path.as_str();
            let trimmed_path = path.trim_start_matches('/');
            matcher.is_match(path) || matcher.is_match(trimmed_path)
        });

        entries.sort_by(|a, b| {
            let a_time = a.updated_at.unwrap_or(0);
            let b_time = b.updated_at.unwrap_or(0);
            b_time.cmp(&a_time).then_with(|| a.path.cmp(&b.path))
        });

        Ok(entries.into_iter().map(|e| e.path).collect())
    }

    pub async fn touch(&self, path: impl AsRef<str>) -> Result<()> {
        let path = normalize_path(path.as_ref())?;
        if path == "/" {
            return Ok(());
        }
        let parent = parent_path(&path).ok_or(FsError::InvalidPath)?;
        self.ensure_dir(&parent).await?;

        match self.get_entry(&path).await? {
            Some(entry) if entry.is_dir => Err(FsError::NotAFile(path)),
            Some(entry) => {
                self.persist_entry(&entry).await?;
                Ok(())
            }
            None => {
                self.create_file(&path, &parent, "").await?;
                Ok(())
            }
        }
    }

    pub async fn write_file(
        &self,
        path: impl AsRef<str>,
        content: impl Into<String>,
    ) -> Result<()> {
        let path = normalize_path(path.as_ref())?;
        if path == "/" {
            return Err(FsError::NotAFile(path));
        }
        let parent = parent_path(&path).ok_or(FsError::InvalidPath)?;
        self.ensure_dir(&parent).await?;

        if let Some(mut entry) = self.get_entry(&path).await? {
            if entry.is_dir {
                return Err(FsError::NotAFile(path));
            }
            entry.content = Some(content.into());
            self.persist_entry(&entry).await?;
        } else {
            self.create_file(&path, &parent, content.into()).await?;
        }
        Ok(())
    }

    pub async fn edit(
        &self,
        path: impl AsRef<str>,
        old: impl AsRef<str>,
        new: impl AsRef<str>,
        replace_all: bool,
    ) -> Result<String> {
        let path = normalize_path(path.as_ref())?;
        let old_str = old.as_ref();
        let new_str = new.as_ref();

        let current = self.cat(&path).await?;

        let (updated, changed) = if old_str.is_empty() {
            let changed = current != new_str;
            (new_str.to_string(), changed)
        } else if replace_all {
            let replaced = current.replace(old_str, new_str);
            let changed = replaced != current;
            (replaced, changed)
        } else if let Some(idx) = current.find(old_str) {
            let mut result =
                String::with_capacity(current.len() + new_str.len().saturating_sub(old_str.len()));
            result.push_str(&current[..idx]);
            result.push_str(new_str);
            result.push_str(&current[idx + old_str.len()..]);
            (result, true)
        } else {
            (current.clone(), false)
        };

        if !changed {
            return Ok(String::new());
        }

        self.write_file(&path, updated.clone()).await?;
        Ok(render_diff(&current, &updated))
    }

    pub async fn mkdir(&self, path: impl AsRef<str>, parents: bool) -> Result<()> {
        let path = normalize_path(path.as_ref())?;
        if path == "/" {
            return if parents {
                Ok(())
            } else {
                Err(FsError::AlreadyExists(path))
            };
        }

        if parents {
            let mut current = String::from("/");
            for segment in path.trim_start_matches('/').split('/') {
                if segment.is_empty() {
                    continue;
                }
                if current != "/" {
                    current.push('/');
                }
                current.push_str(segment);

                match self.get_entry(&current).await? {
                    Some(entry) => {
                        if !entry.is_dir {
                            return Err(FsError::NotADirectory(current));
                        }
                    }
                    None => {
                        let parent = parent_path(&current).unwrap_or("/".to_string());
                        self.create_dir(&current, &parent).await?;
                    }
                }
            }
            return Ok(());
        }

        let parent = parent_path(&path).ok_or(FsError::InvalidPath)?;
        self.ensure_dir(&parent).await?;

        match self.get_entry(&path).await? {
            Some(entry) if entry.is_dir => Err(FsError::AlreadyExists(path)),
            Some(_) => Err(FsError::AlreadyExists(path)),
            None => {
                self.create_dir(&path, &parent).await?;
                Ok(())
            }
        }
    }

    /// Copy a file from `src` to `dest`, overwriting the destination file if it exists.
    /// Destination parent must already exist and be a directory.
    pub async fn cp(&self, src: impl AsRef<str>, dest: impl AsRef<str>) -> Result<()> {
        let src = normalize_path(src.as_ref())?;
        let dest = normalize_path(dest.as_ref())?;

        let content = self.cat(&src).await?;

        if dest == "/" {
            return Err(FsError::NotAFile(dest));
        }
        let parent = parent_path(&dest).ok_or(FsError::InvalidPath)?;
        self.ensure_dir(&parent).await?;

        self.write_file(&dest, content).await
    }

    /// Change directory: resolve `target` relative to `current`, ensure it exists and is a directory.
    /// Returns the normalized new path.
    pub async fn cd(&self, current: &str, target: &str) -> Result<String> {
        let resolved = resolve_relative(current, target)?;
        match self.get_entry(&resolved).await? {
            Some(e) if e.is_dir => Ok(resolved),
            Some(_) => Err(FsError::NotADirectory(resolved)),
            None => Err(FsError::NotFound(resolved)),
        }
    }

    /// Return the normalized path for the current directory.
    pub fn pwd(&self, current: &str) -> Result<String> {
        normalize_path(current)
    }

    async fn require_file(&self, path: &str) -> Result<Entry> {
        let path = normalize_path(path)?;
        match self.get_entry(&path).await? {
            Some(entry) if entry.is_dir => Err(FsError::NotAFile(path)),
            Some(entry) => Ok(entry),
            None => Err(FsError::NotFound(path)),
        }
    }

    async fn ensure_dir(&self, path: &str) -> Result<()> {
        if path == "/" {
            return Ok(());
        }
        match self.get_entry(path).await? {
            Some(entry) if entry.is_dir => Ok(()),
            Some(_) => Err(FsError::NotADirectory(path.to_string())),
            None => Err(FsError::NotFound(path.to_string())),
        }
    }

    async fn children(&self, path: &str) -> Result<Vec<Entry>> {
        let parent = path.to_string();
        let mut res = self
            .db
            .query(format!(
                "SELECT path, name, parent, is_dir, content, updated_at FROM {} WHERE parent = $parent ORDER BY name",
                self.table
            ))
            .bind(("parent", parent))
            .await?;

        let entries: Vec<Entry> = res.take(0)?;
        Ok(entries)
    }

    async fn get_entry(&self, path: &str) -> Result<Option<Entry>> {
        let path_owned = path.to_string();
        let mut res = self
            .db
            .query(format!(
                "SELECT path, name, parent, is_dir, content, updated_at FROM {} WHERE path = $path LIMIT 1",
                self.table
            ))
            .bind(("path", path_owned))
            .await?;
        let entry: Option<Entry> = res.take(0)?;
        Ok(entry)
    }

    async fn create_dir(&self, path: &str, parent: &str) -> Result<()> {
        let path_owned = path.to_string();
        let parent_owned = parent.to_string();
        self.db
            .query(format!(
                "CREATE {} SET path = $path, name = $name, parent = $parent, is_dir = true, content = NONE, updated_at = $updated_at",
                self.table
            ))
            .bind(("path", path_owned))
            .bind(("name", leaf_name(path)))
            .bind(("parent", parent_owned))
            .bind(("updated_at", now_millis()))
            .await?;
        Ok(())
    }

    async fn create_file(
        &self,
        path: &str,
        parent: &str,
        content: impl Into<String>,
    ) -> Result<()> {
        let content = content.into();
        let path_owned = path.to_string();
        let parent_owned = parent.to_string();
        self.db
            .query(format!(
                "CREATE {} SET path = $path, name = $name, parent = $parent, is_dir = false, content = $content, updated_at = $updated_at",
                self.table
            ))
            .bind(("path", path_owned))
            .bind(("name", leaf_name(path)))
            .bind(("parent", parent_owned))
            .bind(("content", content))
            .bind(("updated_at", now_millis()))
            .await?;
        Ok(())
    }

    async fn persist_entry(&self, entry: &Entry) -> Result<()> {
        let path_owned = entry.path.clone();
        let name_owned = entry.name.clone();
        let parent_owned = entry.parent.clone();
        self.db
            .query(format!(
                "UPDATE {} SET content = $content, name = $name, parent = $parent, is_dir = $is_dir, updated_at = $updated_at WHERE path = $path",
                self.table
            ))
            .bind(("path", path_owned))
            .bind(("name", name_owned))
            .bind(("parent", parent_owned))
            .bind(("is_dir", entry.is_dir))
            .bind(("content", entry.content.clone()))
            .bind(("updated_at", now_millis()))
            .await?;
        Ok(())
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn render_diff(old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }

    let diff = TextDiff::from_lines(old, new);
    let mut out = String::from("--- original\n+++ updated\n");

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };

        out.push(sign);
        out.push_str(change.value());
        if !change.value().ends_with('\n') {
            out.push('\n');
        }
    }

    out
}

fn leaf_name(path: &str) -> String {
    if path == "/" {
        return "/".into();
    }
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn parent_path(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let mut parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    parts.pop();
    if parts.is_empty() {
        return Some("/".into());
    }

    let mut parent = parts.join("/");
    if parent.is_empty() {
        parent.push('/');
    } else if !parent.starts_with('/') {
        parent.insert(0, '/');
    }

    Some(parent.replace("//", "/"))
}

fn normalize_path(input: &str) -> Result<String> {
    if input.is_empty() {
        return Err(FsError::InvalidPath);
    }
    let mut components: Vec<String> = Vec::new();
    for comp in input.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                if components.is_empty() {
                    continue;
                }
                components.pop();
            }
            _ => components.push(comp.to_string()),
        }
    }
    let normalized = if components.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", components.join("/"))
    };
    Ok(normalized)
}

fn resolve_relative(base: &str, target: &str) -> Result<String> {
    if target.is_empty() {
        return Err(FsError::InvalidPath);
    }
    if target.starts_with('/') {
        return normalize_path(target);
    }

    let mut combined = String::from(base);
    if !combined.ends_with('/') {
        combined.push('/');
    }
    combined.push_str(target);
    normalize_path(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use surrealdb::engine::local::{Db, Mem};
    use tokio::time::sleep;

    async fn setup_fs() -> Result<SurrealFs<Db>> {
        let db = Surreal::new::<Mem>(()).await?;
        db.use_ns("test").use_db("test").await?;
        Ok(SurrealFs::new(db))
    }

    #[tokio::test]
    async fn touch_and_cat() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/dir", true).await.unwrap();
        fs.touch("/dir/file.txt").await.unwrap();
        fs.write_file("/dir/file.txt", "hello\nworld")
            .await
            .unwrap();
        let content = fs.cat("/dir/file.txt").await.unwrap();
        assert_eq!(content, "hello\nworld");
    }

    #[tokio::test]
    async fn tail_and_nl() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/logs", true).await.unwrap();
        fs.write_file("/logs/app.log", "a\nb\nc\nd").await.unwrap();
        let tail = fs.tail("/logs/app.log", 2).await.unwrap();
        assert_eq!(tail, vec!["c", "d"]);
        let numbered = fs.nl("/logs/app.log", 1).await.unwrap();
        assert_eq!(numbered[0].number, 1);
        assert_eq!(numbered[3].line, "d");
    }

    #[tokio::test]
    async fn read_with_offset_and_limit() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/logs", true).await.unwrap();
        fs.write_file("/logs/app.log", "l1\nl2\nl3\nl4\nl5")
            .await
            .unwrap();

        let middle = fs.read("/logs/app.log", 1, 3).await.unwrap();
        assert_eq!(middle, vec!["l2", "l3", "l4"]);

        let tail = fs.read("/logs/app.log", 4, 10).await.unwrap();
        assert_eq!(tail, vec!["l5"]);

        let empty = fs.read("/logs/app.log", 10, 2).await.unwrap();
        assert!(empty.is_empty());

        let none = fs.read("/logs/app.log", 0, 0).await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn ls_and_grep_recursive() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/code/src", true).await.unwrap();
        fs.write_file("/code/src/main.rs", "fn main() { println!(\"hi\"); }\n")
            .await
            .unwrap();
        fs.write_file("/code/readme.md", "hi there\n")
            .await
            .unwrap();
        let entries = fs.ls("/code").await.unwrap();
        let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
        assert!(names.contains(&"src".to_string()));
        assert!(names.contains(&"readme.md".to_string()));

        let regex = Regex::new("hi").unwrap();
        let matches = fs.grep(&regex, "/code", true).await.unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[tokio::test]
    async fn mkdir_nested_with_parents() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/a/b/c", true).await.unwrap();
        let entries = fs.ls("/a/b").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_dir);
    }

    #[tokio::test]
    async fn mkdir_without_parents_needs_parent() {
        let fs = setup_fs().await.unwrap();
        let err = fs.mkdir("/missing/child", false).await.unwrap_err();
        matches!(err, FsError::NotFound(_));
    }

    #[tokio::test]
    async fn ls_root_lists_children() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/docs", true).await.unwrap();
        fs.write_file("/readme.md", "hello").await.unwrap();

        let entries = fs.ls("/").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"docs"));
        assert!(names.contains(&"readme.md"));

        let dir = entries.iter().find(|e| e.name == "docs").unwrap();
        assert!(dir.is_dir);
    }

    #[tokio::test]
    async fn mkdir_without_parents_fails_when_exists() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/data", true).await.unwrap();
        let err = fs.mkdir("/data", false).await.unwrap_err();
        matches!(err, FsError::AlreadyExists(_));
    }

    #[tokio::test]
    async fn cp_file() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/docs", true).await.unwrap();
        fs.write_file("/docs/src.txt", "copy me").await.unwrap();
        fs.mkdir("/docs/copies", true).await.unwrap();
        fs.cp("/docs/src.txt", "/docs/copies/dest.txt")
            .await
            .unwrap();

        let content = fs.cat("/docs/copies/dest.txt").await.unwrap();
        assert_eq!(content, "copy me");
    }

    #[tokio::test]
    async fn glob_matches_newest_first() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/proj/src", true).await.unwrap();
        fs.mkdir("/proj/tests", true).await.unwrap();

        fs.write_file("/proj/src/main.rs", "main").await.unwrap();
        sleep(Duration::from_millis(5)).await;
        fs.write_file("/proj/src/lib.rs", "lib").await.unwrap();
        sleep(Duration::from_millis(5)).await;
        fs.write_file("/proj/tests/main.rs", "test").await.unwrap();

        let matches = fs.glob("/proj/**/*.rs").await.unwrap();
        assert_eq!(
            matches,
            vec![
                "/proj/tests/main.rs",
                "/proj/src/lib.rs",
                "/proj/src/main.rs",
            ]
        );

        let root_matches = fs.glob("**/*.rs").await.unwrap();
        assert_eq!(root_matches, matches);
    }

    #[tokio::test]
    async fn edit_replaces_first() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/notes", true).await.unwrap();
        fs.write_file("/notes/todo.txt", "alpha beta alpha")
            .await
            .unwrap();

        let diff = fs
            .edit("/notes/todo.txt", "alpha", "ALPHA", false)
            .await
            .unwrap();

        let content = fs.cat("/notes/todo.txt").await.unwrap();
        assert_eq!(content, "ALPHA beta alpha");
        assert!(diff.contains("-alpha beta alpha"));
        assert!(diff.contains("+ALPHA beta alpha"));
    }

    #[tokio::test]
    async fn edit_replaces_all() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/notes", true).await.unwrap();
        fs.write_file("/notes/all.txt", "foo bar foo")
            .await
            .unwrap();

        let diff = fs.edit("/notes/all.txt", "foo", "FOO", true).await.unwrap();

        let content = fs.cat("/notes/all.txt").await.unwrap();
        assert_eq!(content, "FOO bar FOO");
        assert!(diff.contains("-foo bar foo"));
        assert!(diff.contains("+FOO bar FOO"));
    }

    #[tokio::test]
    async fn edit_with_empty_old_overwrites_file() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/notes", true).await.unwrap();
        fs.write_file("/notes/full.txt", "original").await.unwrap();

        let diff = fs
            .edit("/notes/full.txt", "", "hello martin!", false)
            .await
            .unwrap();

        let content = fs.cat("/notes/full.txt").await.unwrap();
        assert_eq!(content, "hello martin!");
        assert!(diff.contains("-original"));
        assert!(diff.contains("+hello martin!"));

        let no_diff = fs
            .edit("/notes/full.txt", "", "hello martin!", false)
            .await
            .unwrap();
        assert!(no_diff.is_empty());
    }

    #[tokio::test]
    async fn cd_and_pwd() {
        let fs = setup_fs().await.unwrap();
        fs.mkdir("/home/user", true).await.unwrap();
        let mut cwd = "/".to_string();

        cwd = fs.cd(&cwd, "home").await.unwrap();
        assert_eq!(cwd, "/home");

        cwd = fs.cd(&cwd, "user").await.unwrap();
        assert_eq!(cwd, "/home/user");

        cwd = fs.cd(&cwd, "..").await.unwrap();
        assert_eq!(cwd, "/home");

        let pwd = fs.pwd(&cwd).unwrap();
        assert_eq!(pwd, "/home");

        let err = fs.cd(&cwd, "nope").await.unwrap_err();
        matches!(err, FsError::NotFound(_));
    }
}
