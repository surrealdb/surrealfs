# surrealfs-ai

Pydantic AI toolset that exposes SurrealFs to an agent so it can read, write, and organize notes inside a SurrealDB-backed virtual filesystem.

## Requirements
- Python 3.11+
- SurrealDB reachable at `ws://localhost:8000` (default Root signin); adjust `src/tools.py` if you use a different endpoint or auth.
- Local build of the `surrealfs_py` Python extension from this repo (Rust bindings via maturin).

## Install (from repo root)

```bash
uv venv
source .venv/bin/activate
uv pip install -e .            # builds surrealfs_py
uv pip install -e python/surrealfs_ai
```

## Quickstart
Run the demo that asks the agent to create and read a file:

```python
import asyncio
from surrealfs_ai import demo

asyncio.run(demo())
```

Host the agent as an HTTP service (defaults to 127.0.0.1:7932):

```python
import uvicorn
from surrealfs_ai import build_chat_agent

agent = build_chat_agent()
app = agent.to_web()
uvicorn.run(app, host="127.0.0.1", port=7932)
```

## Available tools
The toolset mirrors SurrealFs operations:
- `ls` (all/long/recursive/dir_only/human flags)
- `cat`, `tail` (n)
- `write_file`, `edit` (replace_all), `touch`
- `mkdir` (parents), `cp`
- `cd`, `pwd`

Each tool description lives in `src/tool_docs/`; the agent uses them to build richer, self-describing prompts.

## Notes
- Telemetry is handled via `logfire` and auto-instrumented when a token is present; otherwise it stays local.
- The packaged agent uses the Claude Haiku model ID set in `build_chat_agent`; update it there if you want a different provider or model.
