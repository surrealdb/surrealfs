# surrealfs

Rust crate and CLI REPL that expose a SurrealDB-backed virtual filesystem. Default storage is embedded RocksDB at `./demo-db`; set `SURREALFS_REMOTE=1` to use a remote SurrealDB websocket endpoint (root/root, ns=db -> surrealfs/demo). Core commands: `ls`, `cat`, `tail`, `nl`, `grep`, `touch`, `mkdir`, `write_file`, `edit`, `cp`, `cd`, `pwd`; paths are normalized and cannot escape `/`.

## Rust crate & CLI

- Run the demo REPL: `cargo run` or `just run`
- Checks: `cargo check`; tests: `cargo test`
- Crate: `surrealfs` (edition 2024, `rlib` + `cdylib`), async API in `src/lib.rs`

## Python bindings

- Feature flag `python` exposes `PySurrealFs`; see `PYTHON.md` for full instructions
- Quickstart: `uv venv && uv pip install maturin && uv run maturin develop --uv`
- Smoke tests: `uv run pytest`; example: `uv run python examples/python_smoke.py`

### Agent tool example

`python/surrealfs_ai/src/__init__.py` builds a `pydantic_ai` `Agent` with the surrealfs toolset. The `demo()` call creates `/demo/hello.txt` via the virtual filesystem and reads it back; running the module starts a `uvicorn` app so the agent can be used over HTTP.

## Curl piping examples

- Save a URL directly to a path: `curl https://example.com > /pages/example.html`
- Use the pipeline form: `curl https://example.com | write_file /pages/example.html`
- Include headers or data and still redirect: `curl -H "Accept: application/json" https://api.example.com/items > /data/items.json`
