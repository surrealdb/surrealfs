AGENTS PLAYBOOK (surrealfs)
Version: 2026-01-26

Purpose
- Field guide for coding agents in this repo: commands, style, guardrails, local conventions. Treat as the source of truth when automating work.

Repository Snapshot
- Language: Rust (edition 2024).
- Crate type: library + demo bin (async, tokio-based).
- Key deps: surrealdb (=2.4.1, features: kv-mem, kv-rocksdb), tokio, regex, reqwest (0.12, rustls), serde, thiserror, similar.
- Demo REPL in src/main.rs; primary API in src/lib.rs. No workspace members.
- No CI config; keep checks local.

Build / Test / Run
- Full check: `cargo check`
- Full tests: `cargo test`
- Single test (by name): `cargo test <test_name>` (e.g., `cargo test cd_and_pwd`)
- Run demo (loads .env if present): `cargo run` (or `just run` via Justfile)
- Format: `cargo fmt`
- Lint (opt-in): `cargo clippy -- -D warnings`
- Clean: `cargo clean` (rarely needed; prefer incremental builds)
- Examples: none beyond demo bin.

Runtime / Environment
- Default runtime: tokio (multi-thread). Avoid blocking operations in async code.
- REPL chooses backend via `SURREALFS_REMOTE`. If set: ws://127.0.0.1:8000, auth root/root, ns=surrealfs, db=demo. Otherwise RocksDB at ./demo-db (created if missing).
- CLI prints and flushes manually; library remains silent (no prints/logging).
- `.env` is optional; Justfile only wraps `cargo run`.

Source Layout
- src/lib.rs: SurrealFs API (ls, cat, tail, nl, grep, touch, mkdir_p, write_file, edit, cp, cd, pwd), path helpers, error types, public structs.
- src/main.rs: interactive REPL, cwd tracking, arg parsing, curl, ls flags, cd/pwd handling.
- .cursor/rules/: curated Cursor guidance (see below). No Copilot instructions present.
- Justfile: shortcut `just run`.

Dependencies / Features
- surrealdb: remote ws client + local RocksDB; tests use Mem. Table default `fs_entry`.
- reqwest uses rustls; HTTP helpers live only in CLI for curl.
- similar/TextDiff used for edit diffs.
- serde derives on exposed structs for potential bindings.

Imports / Modules
- Group imports std :: third-party :: crate. Avoid unused imports to keep builds warning-free.
- Prefer explicit paths over glob use. Keep `use crate::{...}` small and clear.

Formatting / Style
- Run rustfmt defaults. Aim ~100 columns but trust rustfmt.
- Prefer early returns over deeply nested branches when readable.
- Keep comments minimal; use only for non-obvious intent.
- Stick to ASCII unless existing file uses otherwise.

Naming / Types
- Functions snake_case; structs/enums PascalCase; constants SCREAMING_SNAKE. Command strings mirror Unix names (ls, cat, etc.).
- Public data structs (Entry, NumberedLine, GrepMatch) serde-friendly; fields snake_case.
- Type alias Result<T> = std::result::Result<T, FsError> (use it).
- Prefer `&str`/`String` over PathBuf internally; path logic handled by helpers.

Error Handling
- FsError variants: NotFound, AlreadyExists, NotAFile, NotADirectory, InvalidPath, Http, Surreal.
- Map external errors explicitly (reqwest -> FsError::Http in CLI; surrealdb errors auto-convert via From).
- Messages should include context (path/status) but stay concise.
- Do not panic for recoverable cases; return FsError.

Async Patterns
- Everything async; await directly. Avoid blocking std IO inside async (use tokio IO).
- REPL input via tokio BufReader; maintain responsiveness.
- Avoid spawning unless needed; most ops run in-line.

Path & Filesystem Rules
- Always normalize via `normalize_path` and `resolve_relative`; forbid escaping root (`..` cannot climb past `/`).
- `/` is never a file; operations on root either noop or error NotAFile/NotADirectory appropriately.
- Ensure parent directories exist before writes/touch/mkdir_p. cd must target existing dir.
- cp/edit respect types (dir vs file); maintain trailing path semantics.

SurrealDB Usage
- Construct client via Surreal::new::<RocksDb>/Mem or connect (Any) for remote. Always set namespace/db before use.
- Keep table name `fs_entry` unless intentionally parameterized; `with_table` exists for alt tables.
- Prefer parameter binding in queries; avoid interpolating user input.
- Handle missing entries gracefully; ls on `/` with no entries returns empty vec.

SurrealQL Guidance (from .cursor/rules/surrealql.mdc)
- Record IDs formatted table:id; use parameterized statements instead of string concat.
- MERGE/PATCH available; relationships are non-SQL; respect SurrealQL differences.
- Use bindings for values; avoid raw string interpolation.

SurrealDB Rust Guidance (from .cursor/rules/surrealdb-rust.mdc)
- Examples for Mem vs remote clients; set ns/db each time; root signin required remotely.
- Includes schema snippets, datetime helpers. Follow patterns when adding queries or types.

HTTP / Curl Behavior
- Default method GET. If `-d` present without `-X`, default POST.
- Follow redirects only with `-L`. Headers parsed as `Key: Value` pairs.
- Non-2xx surfaces as FsError::Http with status/message; body printed only when appropriate.

CLI Extension Tips
- To add commands: update REPL match arms, help text, and path resolution via `resolve_cli_path` to honor cwd.
- Keep arg parsing simple; on invalid args, return help_error() and print usage.
- Preserve stdout formatting (ls -h uses base 1024 sizes; nl right-align numbers width 4).

Testing Guidance
- Unit tests live in src/lib.rs; use in-memory engine (`Surreal::new::<Mem>(())`) with ns/db "test".
- Avoid network in tests. Exercise library directly rather than stdin parsing for CLI behavior.
- Example targeted test: `cargo test ls_and_grep_recursive`.
- Keep tests deterministic; avoid system time/env dependence unless explicitly set.

Tooling / CI
- No enforced CI. Before commits run `cargo fmt && cargo test` (optionally clippy with -D warnings).
- If adding clippy/config, document commands here and keep warnings as errors for consistency.

Security / Secrets
- No real secrets tracked. Demo remote creds are root/root only for local dev. Do not commit real credentials or tokens.
- Avoid writing secrets to repo; prefer env vars when adding integrations.

Performance Notes
- Surreal queries are simple create/select/update; no pagination yet. Consider streaming if large results are added later.
- Recursive ls uses DFS stack; watch for deep recursion but tree model means no cycles.
- Avoid unnecessary cloning of large strings; reuse references when possible.

Dependency Hygiene
- Pin compatible versions; if bumping surrealdb or reqwest, ensure feature flags (kv-mem, kv-rocksdb, rustls) remain correct.
- After `cargo update -p <crate>`, re-check imports/types in main.rs and lib.rs.

Release / Versioning
- Crate version 0.1.0. No release process defined; tag manually if needed.

Behavioral Guardrails
- Avoid destructive git commands; never reset user changes. Do not amend commits unless explicitly asked.
- Keep outputs concise; library stays silent, CLI handles prints. Avoid non-ASCII unless file already uses it.
- Honor cwd semantics and path resolvers for all file operations.

Examples / Snippets
- Embedded in-memory client:
  ```rust
  let db = Surreal::new::<surrealdb::engine::local::Mem>(()).await?;
  db.use_ns("demo").use_db("demo").await?;
  let fs = SurrealFs::new(db);
  ```
- Single test: `cargo test ls_and_grep_recursive`
- Run REPL with remote DB: `SURREALFS_REMOTE=1 cargo run`

Common Pitfalls
- Forgetting to set namespace/database on Surreal connection -> runtime errors.
- Allowing `..` to escape root -> reject via normalize_path.
- Treating `/` like a file -> should error/ignore appropriately.
- curl without -o/-O prints body; may be large.
- Missing flush on prompts leads to hidden REPL prompt; keep `stdout().flush()`.

Help / Docs
- REPL help lists commands/flags; update when adding features.
- Keep this file in sync with new commands, dependencies, or tooling changes.

End of AGENTS.md
