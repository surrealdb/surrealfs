#![cfg(feature = "python")]

use std::fmt::Write as FmtWrite;
use std::sync::Mutex;

use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyType;
use regex::Regex;
use surrealdb::Surreal;
use surrealdb::engine::any::connect;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::opt::auth::Root;
use tokio::runtime::Runtime;

use crate::{Entry, FsError, SurrealFs};

create_exception!(surrealfs_py, SurrealFsError, pyo3::exceptions::PyException);

#[derive(Clone, Copy, Default)]
struct LsOptions {
    all: bool,
    long: bool,
    recursive: bool,
    dir_only: bool,
    human: bool,
}

enum FsInner {
    Remote(SurrealFs<surrealdb::engine::any::Any>),
    Local(SurrealFs<Db>),
}

impl FsInner {
    async fn ls(&self, path: &str) -> crate::Result<Vec<Entry>> {
        match self {
            FsInner::Remote(fs) => fs.ls(path).await,
            FsInner::Local(fs) => fs.ls(path).await,
        }
    }

    async fn cat(&self, path: &str) -> crate::Result<String> {
        match self {
            FsInner::Remote(fs) => fs.cat(path).await,
            FsInner::Local(fs) => fs.cat(path).await,
        }
    }

    async fn tail(&self, path: &str, n: usize) -> crate::Result<Vec<String>> {
        match self {
            FsInner::Remote(fs) => fs.tail(path, n).await,
            FsInner::Local(fs) => fs.tail(path, n).await,
        }
    }

    async fn read(&self, path: &str, offset: usize, limit: usize) -> crate::Result<Vec<String>> {
        match self {
            FsInner::Remote(fs) => fs.read(path, offset, limit).await,
            FsInner::Local(fs) => fs.read(path, offset, limit).await,
        }
    }

    async fn nl(&self, path: &str, start_at: usize) -> crate::Result<Vec<crate::NumberedLine>> {
        match self {
            FsInner::Remote(fs) => fs.nl(path, start_at).await,
            FsInner::Local(fs) => fs.nl(path, start_at).await,
        }
    }

    async fn grep(
        &self,
        pattern: &Regex,
        path: &str,
        recursive: bool,
    ) -> crate::Result<Vec<crate::GrepMatch>> {
        match self {
            FsInner::Remote(fs) => fs.grep(pattern, path, recursive).await,
            FsInner::Local(fs) => fs.grep(pattern, path, recursive).await,
        }
    }

    async fn touch(&self, path: &str) -> crate::Result<()> {
        match self {
            FsInner::Remote(fs) => fs.touch(path).await,
            FsInner::Local(fs) => fs.touch(path).await,
        }
    }

    async fn write_file(&self, path: &str, content: String) -> crate::Result<()> {
        match self {
            FsInner::Remote(fs) => fs.write_file(path, content).await,
            FsInner::Local(fs) => fs.write_file(path, content).await,
        }
    }

    async fn edit(
        &self,
        path: &str,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> crate::Result<String> {
        match self {
            FsInner::Remote(fs) => fs.edit(path, old, new, replace_all).await,
            FsInner::Local(fs) => fs.edit(path, old, new, replace_all).await,
        }
    }

    async fn mkdir(&self, path: &str, parents: bool) -> crate::Result<()> {
        match self {
            FsInner::Remote(fs) => fs.mkdir(path, parents).await,
            FsInner::Local(fs) => fs.mkdir(path, parents).await,
        }
    }

    async fn cp(&self, src: &str, dest: &str) -> crate::Result<()> {
        match self {
            FsInner::Remote(fs) => fs.cp(src, dest).await,
            FsInner::Local(fs) => fs.cp(src, dest).await,
        }
    }

    async fn glob(&self, pattern: &str) -> crate::Result<Vec<String>> {
        match self {
            FsInner::Remote(fs) => fs.glob(pattern).await,
            FsInner::Local(fs) => fs.glob(pattern).await,
        }
    }

    async fn cd(&self, current: &str, target: &str) -> crate::Result<String> {
        match self {
            FsInner::Remote(fs) => fs.cd(current, target).await,
            FsInner::Local(fs) => fs.cd(current, target).await,
        }
    }

    fn pwd(&self, current: &str) -> crate::Result<String> {
        match self {
            FsInner::Remote(fs) => fs.pwd(current),
            FsInner::Local(fs) => fs.pwd(current),
        }
    }
}

#[pyclass(module = "surrealfs_py")]
pub struct PySurrealFs {
    rt: Runtime,
    cwd: Mutex<String>,
    fs: FsInner,
}

#[pymethods]
impl PySurrealFs {
    #[classmethod]
    pub fn connect_ws(
        _cls: &PyType,
        url: &str,
        namespace: Option<&str>,
        database: Option<&str>,
    ) -> PyResult<Self> {
        let ns = namespace.unwrap_or("surrealfs");
        let db_name = database.unwrap_or("demo");

        let rt = Runtime::new().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let fs = rt
            .block_on(async move {
                let db = connect(url).await?;
                db.signin(Root {
                    username: "root",
                    password: "root",
                })
                .await?;
                db.use_ns(ns).use_db(db_name).await?;
                Ok::<_, FsError>(SurrealFs::new(db))
            })
            .map_err(to_py_err)?;

        Ok(Self {
            rt,
            cwd: Mutex::new("/".to_string()),
            fs: FsInner::Remote(fs),
        })
    }

    #[classmethod]
    pub fn mem(_cls: &PyType, namespace: Option<&str>, database: Option<&str>) -> PyResult<Self> {
        let ns = namespace.unwrap_or("surrealfs");
        let db_name = database.unwrap_or("demo");

        let rt = Runtime::new().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let fs = rt
            .block_on(async move {
                let db = Surreal::new::<Mem>(()).await?;
                db.use_ns(ns).use_db(db_name).await?;
                Ok::<_, FsError>(SurrealFs::new(db))
            })
            .map_err(to_py_err)?;

        Ok(Self {
            rt,
            cwd: Mutex::new("/".to_string()),
            fs: FsInner::Local(fs),
        })
    }

    pub fn ls(
        &self,
        path: Option<&str>,
        all: Option<bool>,
        long: Option<bool>,
        recursive: Option<bool>,
        dir_only: Option<bool>,
        human: Option<bool>,
    ) -> PyResult<String> {
        let opts = LsOptions {
            all: all.unwrap_or(false),
            long: long.unwrap_or(false),
            recursive: recursive.unwrap_or(false),
            dir_only: dir_only.unwrap_or(false),
            human: human.unwrap_or(false),
        };

        let resolved = self.resolve_path(path.unwrap_or("/"))?;
        self.rt
            .block_on(format_ls(&self.fs, &resolved, opts))
            .map_err(to_py_err)
    }

    pub fn cat(&self, path: &str) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        self.rt.block_on(self.fs.cat(&resolved)).map_err(to_py_err)
    }

    pub fn tail(&self, path: &str, n: Option<usize>) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        let count = n.unwrap_or(10);
        let lines = self
            .rt
            .block_on(self.fs.tail(&resolved, count))
            .map_err(to_py_err)?;
        Ok(join_lines(lines))
    }

    pub fn read(&self, path: &str, offset: usize, limit: usize) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        let lines = self
            .rt
            .block_on(self.fs.read(&resolved, offset, limit))
            .map_err(to_py_err)?;
        Ok(join_lines(lines))
    }

    pub fn nl(&self, path: &str, start: Option<usize>) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        let start_at = start.unwrap_or(1);
        let lines = self
            .rt
            .block_on(self.fs.nl(&resolved, start_at))
            .map_err(to_py_err)?;
        let mut out = String::new();
        for l in lines {
            let _ = writeln!(&mut out, "{:>4}  {}", l.number, l.line);
        }
        Ok(out)
    }

    pub fn grep(&self, pattern: &str, path: &str, recursive: Option<bool>) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        let recursive = recursive.unwrap_or(false);
        let re = Regex::new(pattern).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let matches = self
            .rt
            .block_on(self.fs.grep(&re, &resolved, recursive))
            .map_err(to_py_err)?;
        let mut out = String::new();
        for m in matches {
            let _ = writeln!(&mut out, "{}:{}: {}", m.path, m.line_number, m.line);
        }
        Ok(out)
    }

    pub fn touch(&self, path: &str) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        self.rt
            .block_on(self.fs.touch(&resolved))
            .map_err(to_py_err)?;
        Ok(String::new())
    }

    pub fn write_file(&self, path: &str, content: &str) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        self.rt
            .block_on(self.fs.write_file(&resolved, content.to_string()))
            .map_err(to_py_err)?;
        Ok(String::new())
    }

    pub fn edit(
        &self,
        path: &str,
        old: &str,
        new: &str,
        replace_all: Option<bool>,
    ) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        self.rt
            .block_on(
                self.fs
                    .edit(&resolved, old, new, replace_all.unwrap_or(false)),
            )
            .map_err(to_py_err)
    }

    pub fn mkdir(&self, path: &str, parents: Option<bool>) -> PyResult<String> {
        let resolved = self.resolve_path(path)?;
        self.rt
            .block_on(self.fs.mkdir(&resolved, parents.unwrap_or(false)))
            .map_err(to_py_err)?;
        Ok(String::new())
    }

    pub fn cp(&self, src: &str, dest: &str) -> PyResult<String> {
        let resolved_src = self.resolve_path(src)?;
        let resolved_dest = self.resolve_path(dest)?;
        self.rt
            .block_on(self.fs.cp(&resolved_src, &resolved_dest))
            .map_err(to_py_err)?;
        Ok(String::new())
    }

    pub fn cd(&self, target: &str) -> PyResult<String> {
        let current = self.current_cwd();
        let resolved = self
            .rt
            .block_on(self.fs.cd(&current, target))
            .map_err(to_py_err)?;
        if let Ok(mut guard) = self.cwd.lock() {
            *guard = resolved.clone();
        }
        Ok(String::new())
    }

    pub fn pwd(&self) -> PyResult<String> {
        let current = self.current_cwd();
        let path = self.fs.pwd(&current).map_err(to_py_err)?;
        Ok(format!("{}\n", path))
    }

    pub fn glob(&self, pattern: &str) -> PyResult<String> {
        let resolved = self.resolve_path(pattern)?;
        let paths = self
            .rt
            .block_on(self.fs.glob(&resolved))
            .map_err(to_py_err)?;
        Ok(join_lines(paths))
    }
}

#[pymodule]
fn surrealfs_py(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<PySurrealFs>()?;
    m.add("SurrealFsError", _py.get_type::<SurrealFsError>())?;
    Ok(())
}

fn to_py_err(err: FsError) -> PyErr {
    SurrealFsError::new_err(err.to_string())
}

fn join_lines(lines: Vec<String>) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        let mut out = lines.join("\n");
        out.push('\n');
        out
    }
}

fn format_entry(entry: &Entry, opts: LsOptions) -> String {
    if opts.long {
        let kind = if entry.is_dir { 'd' } else { '-' };
        let size = if entry.is_dir {
            0
        } else {
            entry.content.as_ref().map(|c| c.len()).unwrap_or(0)
        };
        if opts.human {
            let (val, unit) = human_size(size as f64);
            format!("{} {:>6.1}{} {}", kind, val, unit, entry.name)
        } else {
            format!("{} {:>8} {}", kind, size, entry.name)
        }
    } else {
        let suffix = if entry.is_dir { "/" } else { "" };
        format!("{}{}", entry.name, suffix)
    }
}

fn human_size(bytes: f64) -> (f64, &'static str) {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T", "P"];
    if bytes < 1.0 {
        return (bytes, "B");
    }
    let mut value = bytes;
    let mut idx = 0;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    (value, UNITS[idx])
}

fn should_show(entry: &Entry, opts: LsOptions) -> bool {
    if !opts.all && entry.name.starts_with('.') {
        return false;
    }
    if opts.dir_only && !entry.is_dir {
        return false;
    }
    true
}

fn resolve_cli_path(current: &str, input: &str) -> crate::Result<String> {
    if input.starts_with('/') {
        crate::normalize_path(input)
    } else {
        crate::resolve_relative(current, input)
    }
}

async fn format_ls(fs: &FsInner, path: &str, opts: LsOptions) -> crate::Result<String> {
    if opts.recursive {
        let mut out = String::new();
        let mut stack = vec![path.to_string()];
        while let Some(p) = stack.pop() {
            let entries = fs.ls(&p).await?;
            let display_entries: Vec<_> = entries.iter().filter(|e| should_show(e, opts)).collect();
            for e in &display_entries {
                let _ = writeln!(&mut out, "{}", format_entry(e, opts));
                if e.is_dir {
                    stack.push(e.path.clone());
                }
            }
        }
        Ok(out)
    } else {
        let entries = fs.ls(path).await?;
        let mut out = String::new();
        for e in entries.into_iter().filter(|e| should_show(e, opts)) {
            let _ = writeln!(&mut out, "{}", format_entry(&e, opts));
        }
        Ok(out)
    }
}

impl PySurrealFs {
    fn resolve_path(&self, input: &str) -> PyResult<String> {
        let current = self.current_cwd();
        resolve_cli_path(&current, input).map_err(to_py_err)
    }

    fn current_cwd(&self) -> String {
        self.cwd
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| "/".to_string())
    }
}
