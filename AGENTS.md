AGENTS PLAYBOOK (surrealfs)
Version: 2026-01-26

Purpose
- Field guide for coding agents in this repo: commands, style, guardrails, and Cursor rules. Treat as the source of truth when automating work.

Repository Snapshot
- Language: Rust (edition 2024); single crate with library + async demo bin.
- Key deps: surrealdb (=2.4.1; features kv-mem, kv-rocksdb), tokio, regex, reqwest 0.12 (rustls), serde, thiserror, similar.
- Demo REPL in src/main.rs; primary API in src/lib.rs. No workspace members. No CI; run checks locally.
- Justfile present (wraps cargo run). No Copilot instructions. Cursor rules live in .cursor/rules/.

Build / Lint / Test
- Check: `cargo check`
- Format: `cargo fmt`
- Lint (opt-in): `cargo clippy -- -D warnings`
- Full test suite: `cargo test`
- Single test: `cargo test <name>` (e.g., `cargo test cd_and_pwd` or `cargo test ls_and_grep_recursive`)
- Run demo: `cargo run` or `just run` (loads .env if present)
- Clean (rare): `cargo clean` (prefer incremental builds)

Runtime / Environment
- Tokio multi-thread runtime; avoid blocking std IO in async contexts.
- Backend selection via `SURREALFS_REMOTE`:
  - Unset: RocksDB at ./demo-db (auto-created).
  - Set: remote ws://127.0.0.1:8000 with auth root/root, ns=surrealfs, db=demo.
- CLI handles prints and flushes; library stays silent (no logging by default).
- `.env` optional; only read by demo run wrapper.

Source Layout
- src/lib.rs: SurrealFs API (ls, cat, tail, nl, grep, touch, mkdir, write_file, edit, cp, cd, pwd), path helpers, error types, public structs.
- src/main.rs: REPL loop, cwd tracking, arg parsing, curl command, ls flags, cd/pwd handling.
- .cursor/rules/: surrealql.mdc and surrealdb-rust.mdc (must honor). No .github/copilot-instructions.md.
- Justfile: shortcut `just run`.

Imports / Modules
- Order imports: std :: third-party :: crate. Keep `use crate::{...}` small and explicit.
- Avoid glob imports; remove unused imports to keep builds warning-free.

Formatting / Style
- Use rustfmt defaults (~100 cols). Prefer early returns over deep nesting when clearer.
- Comments only for non-obvious intent. Default to ASCII unless the file already uses non-ASCII.
- Keep control flow straightforward; avoid clever macro indirection.

Naming / Types
- Functions snake_case; structs/enums PascalCase; constants SCREAMING_SNAKE. CLI commands mirror Unix names (ls, cat, etc.).
- Public structs (Entry, NumberedLine, GrepMatch) keep serde-friendly snake_case fields.
- Use `type Result<T> = std::result::Result<T, FsError>` throughout.
- Prefer `&str`/`String` internally; path logic lives in helpers (PathBuf rarely needed outside them).

Error Handling
- FsError variants: NotFound, AlreadyExists, NotAFile, NotADirectory, InvalidPath, Http, Surreal.
- Map external errors explicitly: reqwest -> FsError::Http in CLI; surrealdb errors via From. Provide concise context (path/status), no panics for recoverable cases.
- CLI should surface help errors via help_error() with usage when args are invalid.

Async Patterns
- Everything async; await directly. Avoid blocking IO; use tokio IO (BufReader for stdin).
- Prefer in-line execution; spawn tasks only when concurrency is required.
- Maintain REPL responsiveness; flush prompts to stdout explicitly.

Path & Filesystem Rules
- Always normalize via `normalize_path` + `resolve_relative`; forbid escaping root (`..` cannot climb past `/`).
- `/` is never a file; root ops either noop or return NotAFile/NotADirectory appropriately.
- Ensure parent dirs exist before writes/touch/mkdir; cd targets must exist. cp/edit respect file vs dir and trailing semantics.

SurrealDB Usage
- Build clients with Surreal::new::<RocksDb>/Mem or Any (remote). Immediately set namespace/db. Embedded needs no auth; remote requires Root signin (root/root).
- Default table `fs_entry`; `with_table` exists for alternates. Handle missing entries gracefully (ls on `/` may return empty vec).
- Prefer parameter binding; never interpolate user input into queries.

Cursor Rule Highlights: SurrealQL (.cursor/rules/surrealql.mdc)
- SurrealQL is not ANSI SQL. Record IDs use `table:id`; relationships use `->`, `<-`, `<->`.
- Use parameterized selects with `type::table($table)` and user bindings; avoid raw concatenation.
- Update modes: replace (default), MERGE, PATCH. Prefer MERGE/PATCH to preserve existing data.
- Use RELATE to create graph edges. Favor specific record IDs when known. Live queries exist but are unused here.

Cursor Rule Highlights: SurrealDB Rust (.cursor/rules/surrealdb-rust.mdc)
- Remote example: Any client to ws://localhost:8000, signin Root { username: "root", password: "root" }, then use_ns/use_db.
- Embedded example: `Surreal::new::<Mem>(()).await?; db.use_ns("namespace").use_db("database").await?;`.
- Schema snippet SCHEMA_SQL shows overwrite fields/time defaults; use RecordId helpers. PatchOp supports replace/remove (e.g., time.deleted_at). Chrono datetimes serialize via helpers into surrealdb::sql::Datetime.

HTTP / Curl Behavior
- Default method GET; `-d` without `-X` implies POST. Follow redirects only with `-L`.
- Headers parsed as `Key: Value`. Non-2xx -> FsError::Http with status/message; print body when appropriate.

CLI Extension Tips
- Adding commands: update REPL match arms, help text, and `resolve_cli_path` to honor cwd.
- Keep arg parsing simple; on invalid args, return help_error() and print usage.
- Preserve output formatting: ls -h uses base 1024 sizes; nl right-align numbers width 4.

Testing Guidance
- Tests live in src/lib.rs; use in-memory engine `Surreal::new::<Mem>(())` with ns/db "test". Avoid network.
- Exercise library functions directly rather than REPL stdin parsing for behavior coverage.
- Keep tests deterministic; avoid ambient time/env dependence unless explicitly set. Example targeted test: `cargo test ls_and_grep_recursive`.

Tooling / CI
- No enforced CI. Before commits run `cargo fmt && cargo test`; optionally add `cargo clippy -- -D warnings`.
- If introducing clippy config or other tools, document commands here and treat warnings as errors for consistency.

Security / Secrets
- No real secrets tracked. Demo remote creds root/root only; do not commit real credentials or tokens. Prefer env vars for integrations.
- Do not write secrets to repo; scrub outputs that could leak sensitive data.

Performance Notes
- Surreal queries are simple create/select/update; no pagination yet—consider streaming if large results appear.
- Recursive ls uses DFS stack; be mindful of deep trees (no cycles expected). Avoid unnecessary cloning of large strings.

Dependency Hygiene
- Maintain pinned versions; if bumping surrealdb/reqwest, ensure features (kv-mem, kv-rocksdb, rustls) remain correct.
- After `cargo update -p <crate>`, re-check imports/types in main.rs and lib.rs for breaking changes.

Release / Versioning
- Crate version 0.1.0. No formal release process; tag manually if needed.

Behavioral Guardrails
- Avoid destructive git commands; never reset user changes. Do not amend commits unless explicitly asked.
- Keep outputs concise; library avoids prints, CLI owns stdout/stderr. Stick to ASCII unless file already uses otherwise.
- Honor cwd semantics and path resolvers for all file operations.

Git / Workflow
- Assume worktree may be dirty; never revert user edits. Stage only relevant changes; avoid force pushes.
- Respect import ordering and rustfmt before commits. Keep diffs minimal; do not auto-format unrelated files.
- When adding tests, prefer focused cases in src/lib.rs using Mem engine and ns/db "test".

Logging / IO
- Library should stay quiet; CLI handles user-facing output. Avoid println! in lib except tests.
- Flush stdout after prompts to keep REPL usable. Use stderr for errors in CLI paths when needed.

Examples / Snippets
- Embedded client:
  ```rust
  let db = Surreal::new::<surrealdb::engine::local::Mem>(()).await?;
  db.use_ns("demo").use_db("demo").await?;
  let fs = SurrealFs::new(db);
  ```
- Run REPL against remote: `SURREALFS_REMOTE=1 cargo run`
- Single test run: `cargo test ls_and_grep_recursive`

Common Pitfalls
- Forgetting to set namespace/database on Surreal connection -> runtime errors.
- Allowing `..` to escape root -> reject via normalize_path.
- Treating `/` like a file -> should error/ignore appropriately.
- curl without -o/-O prints body; may be large. Missing flush on prompts hides REPL prompt; always flush stdout.

Help / Docs
- REPL help lists commands/flags; update when adding features. Keep this file current with new commands/dependencies/tooling.

End of AGENTS.md
