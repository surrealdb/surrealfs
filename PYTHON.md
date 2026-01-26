# Python bindings for surrealfs

## Build and develop locally (uv + maturin)

```bash
uv venv
uv pip install maturin
uv run maturin develop --uv
```

## Run the Python smoke test

```bash
uv run pytest
```

## Example

```bash
uv run python examples/python_smoke.py
```

The bindings expose `PySurrealFs`, which mirrors the CLI commands and returns stdout-style strings. Use `PySurrealFs.mem()` for an in-memory SurrealDB or `PySurrealFs.connect_ws("ws://127.0.0.1:8000")` for remote (root/root, ns=db default to surrealfs/demo).

### pydantic-ai tools

`examples/pydantic_ai_tools.py` shows how to wrap `PySurrealFs` methods with `Tool.from_schema` so they can be used by pydantic-ai agents. The example uses `PySurrealFs.connect_ws("ws://localhost:8000")`; swap to `mem()` if you prefer the in-memory backend.
