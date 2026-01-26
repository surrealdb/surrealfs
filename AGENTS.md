AGENTS PLAYBOOK (surrealfs)
Version: 2026-01-26

Purpose
- Field guide for coding agents working in this repo. Focus on commands, style, guardrails, and local conventions.

Repository Snapshot
- Language: Rust (edition 2024).
- Crate type: library + demo bin (async, tokio-based).
- Key deps: surrealdb (=2.4.1, features: kv-mem, kv-rocksdb), tokio, regex, reqwest (0.12, rustls), serde, thiserror.
- Demo REPL in src/main.rs; primary API in src/lib.rs.

Build / Test / Run
- Full check: `cargo check`
- Full tests: `cargo test`
- Single test (by name): `cargo test <test_name>` (e.g., `cargo test cd_and_pwd`)
- Run demo (respecting .env): `cargo run` (or `just run` if using Justfile; loads .env)
- Format (if needed): `cargo fmt`
- Lint (none configured); prefer `cargo clippy -- -D warnings` if you add clippy.

Runtime Notes
- REPL chooses remote vs local based on `SURREALFS_REMOTE` env. Remote: ws://127.0.0.1:8000 root/root ns=surrealfs db=demo. Local: RocksDB under ./demo-db.
- Async everywhere; tokio macros enabled.

Source Layout
- src/lib.rs: SurrealFs API (ls, cat, tail, nl, grep, touch, mkdir_p, write_file, cp, cd, pwd). Path normalization helpers, error types.
- src/main.rs: Interactive CLI (tracks cwd), commands mirror API plus curl, ls options, cd/pwd.
- Justfile: `just run` (loads .env, calls cargo run).
- .cursor/rules: surrealdb-rust.mdc and surrealql.mdc provide guidance; see below.

Coding Conventions
- Imports: group std, third-party, crate; keep unused imports out to avoid warnings. Prefer explicit imports over glob.
- Error handling: use Result<T, FsError>. Convert external errors via FsError variants; currently only Surreal and Http variants in lib. In CLI, map reqwest errors to FsError::Http.
- Paths: normalize via `normalize_path` and `resolve_relative`; never allow escaping root. Use these helpers for any path handling.
- Filesystem API: treat `/` specially (cannot be file). Ensure parent dirs exist before writing. cd must target existing directory.
- Async: use `.await` directly; avoid blocking calls. Use tokio IO for stdin.
- Data types: Entry/NumberedLine/GrepMatch are serde-friendly structs. Keep public structs camel-case; fields snake_case.
- Naming: functions snake_case; structs PascalCase; constants SCREAMING_SNAKE if added. Keep command names consistent with Unix analogs.
- Formatting: rustfmt defaults; prefer 100ish columns but follow rustfmt.
- Pattern matching: exhaustively handle command args; on invalid args in CLI, print help.
- Logging/prints: CLI prints via println!/print!; library should not print.
- HTTP (curl): default GET; if -d used and no -X, default POST. Follow redirects only with -L. Headers parsed as `Key: Value`. Non-2xx -> FsError::Http.
- Size printing: ls -h uses base 1024.

Cursor Rules (from .cursor/rules)
- surrealdb-rust.mdc: shows SurrealDB Rust usage patterns, in-memory vs remote examples, schema definition snippets, datetime serialization helpers.
- surrealql.mdc: SurrealQL guidance; record IDs table:id, parameterized queries, MERGE/PATCH, relationships, non-SQL differences, example statements. Remember SurrealQL != SQL.
- Apply these as reference; not mandatory unless relevant code touches SurrealDB queries.

Style for SurrealDB Usage
- Connect via Surreal::new::<Any>/connect for remote; Surreal::new::<RocksDb>/Mem for local. Set namespace/db explicitly. Root signin required for remote.
- Prefer parameter binding in queries; avoid string interpolation of user data.
- Keep table name `fs_entry` unless intentionally parameterized.

Testing Guidance
- Unit tests live in src/lib.rs. Use in-memory engine (Mem) and ns/db "test". Avoid external network in tests.
- When adding tests for CLI-adjacent logic, prefer exercising underlying library instead of stdin parsing.

CI/Tooling
- No CI config present; run cargo test before commits. If adding clippy/format checks, document commands here.

Behavioral Guardrails
- Do not introduce non-ASCII unless required.
- Avoid destructive git commands; never reset user changes.
- Keep REPL responsive: avoid long network calls; report HTTP status even on errors.
- For new commands, ensure they honor cwd and use path resolvers.

Extending CLI
- Add new commands by updating REPL match, help text, and path resolution via `resolve_cli_path`.
- Keep options parsing simple; on invalid args, call print_help.

Error Messages
- FsError variants: NotFound, AlreadyExists, NotAFile, NotADirectory, InvalidPath, Surreal, Http.
- Prefer precise context in Http errors (status, message).

Performance Notes
- SurrealQL queries currently simple selects/creates/updates; no pagination. For large listings, consider streaming later.
- ls recursive uses DFS stack; no cycle detection needed (tree model), but watch for deep recursion via stack vector.

Security/Secrets
- No secrets in repo. Remote creds are root/root for demo only. Do not commit real credentials.

Adding Dependencies
- Pin compatible versions; ensure features match (reqwest with rustls). Run `cargo update -p <crate>` cautiously; re-check main.rs imports.

Release/Versioning
- Crate version 0.1.0. No release process defined.

Examples
- Connect embedded:
  ```rust
  let db = Surreal::new::<surrealdb::engine::local::Mem>(()).await?;
  db.use_ns("demo").use_db("demo").await?;
  let fs = SurrealFs::new(db);
  ```
- Single test run: `cargo test ls_and_grep_recursive`

Common Pitfalls
- Forgetting to set ns/db on Surreal connection -> errors.
- Using absolute paths incorrectly: always normalize; forbid `..` beyond root.
- curl without -o/-O prints body; may be large—consider in future improvements.

Help/Docs
- REPL help shows available commands and flags; update it when adding commands.
- Keep this file in sync with new commands/features.

End of AGENTS.md
