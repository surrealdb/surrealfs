# surrealfs

User-facing Python package `surrealfs-ai` that builds Pydantic AI agents on top of the SurrealFs virtual filesystem, backed by SurrealDB. The Rust crate `surrealfs` is the core engine (async API + CLI REPL) used by the Python bindings.

## Python agent (main entry point)

- Requirements: Python 3.11+, SurrealDB at `ws://localhost:8000` (Root signin), local build of the `surrealfs_py` extension from this repo.
- Install from repo root:
  ```bash
  uv venv
  source .venv/bin/activate
  uv pip install -e .            # builds surrealfs_py
  uv pip install -e python/surrealfs_ai
  ```
- Quickstart demo (agent creates and reads a file):
  ```bash
  uv run python - <<'PY'
  import asyncio
  from surrealfs_ai import demo

  asyncio.run(demo())
  PY
  ```
- Host the agent over HTTP (default 127.0.0.1:7932):
  ```bash
  uv run python - <<'PY'
  import uvicorn
  from surrealfs_ai import build_chat_agent

  agent = build_chat_agent()
  app = agent.to_web()
  uvicorn.run(app, host="127.0.0.1", port=7932)
  PY
  ```
- Available tools mirror SurrealFs: `ls` (all/long/recursive/dir_only/human), `cat`, `tail`, `write_file`, `edit`, `touch`, `mkdir`, `cp`, `cd`, `pwd`. Tool docs live in `python/surrealfs_ai/src/tool_docs/`.
- Telemetry uses `logfire` when a token is present; otherwise remains local. The packaged agent defaults to the Claude Haiku model set in `build_chat_agent`.

## Rust crate & CLI (core)

- Run the demo REPL: `cargo run` or `just run`
- Checks: `cargo check`; tests: `cargo test`
- Crate: `surrealfs` (edition 2024, `rlib` + `cdylib`), async API in `src/lib.rs`
- Default storage is embedded RocksDB at `./demo-db`; set `SURREALFS_REMOTE=1` for remote SurrealDB websocket (root/root, ns=db -> surrealfs/demo). Paths are normalized and cannot escape `/`. Core commands: `ls`, `cat`, `tail`, `nl`, `grep`, `touch`, `mkdir`, `write_file`, `edit`, `cp`, `cd`, `pwd`.

## Curl piping examples

- Save a URL directly to a path: `curl https://example.com > /pages/example.html`
- Use the pipeline form: `curl https://example.com | write_file /pages/example.html`
- Include headers or data and still redirect: `curl -H "Accept: application/json" https://api.example.com/items > /data/items.json`
