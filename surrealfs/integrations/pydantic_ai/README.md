# SurrealFS for pydantic-ai

The filesystem as a `FunctionToolset`. Needs the extra:

```bash
pip install "surrealfs[pydantic-ai]"
```

```python
from pydantic_ai import Agent
from surrealfs import SurrealFs
from surrealfs.integrations.pydantic_ai import build_fs_toolset
from surrealfs.tools import ToolContext

agent = Agent(
    "anthropic:claude-opus-5",  # the provider prefix is required by pydantic-ai
    deps_type=ToolContext,
    toolsets=[build_fs_toolset()],
)

await agent.run("Tidy up my notes", deps=ToolContext(fs=SurrealFs(db, user="alice")))
```

The agent's deps carry the filesystem. The default `get_context` accepts a
`ToolContext` directly, or any deps object exposing one as `.fs_context` — pass
`get_context=` to `build_fs_toolset()` for anything else.

Recoverable mistakes — a wrong path, text that is not in the file — are raised as
`ModelRetry` so the agent corrects itself. Real failures (a dead connection, a
malformed query) propagate: they are our bug, not the model's, and a retry would
only burn a turn.

## Large files

`read_bytes` replies with JSON rather than bare base64: `data_base64`, the file's
`size`, the `offset` it started at, and `next_offset` (`null` at the end of the
file). It returns at most 64 KiB per call, so a large or chunked file is read by
passing `next_offset` back as `offset`. There is no streaming write tool:
`write_bytes` carries its whole file as base64 in one call, and a large file is
written from Python with `SurrealFs.write_stream`.

## Hybrid search

`search` is full-text only unless you opt in, because match-by-meaning needs an
embedder that only you can supply:

```python
toolsets = [build_fs_toolset(semantic=True)]
...
deps = ToolContext(fs=SurrealFs(db, user="alice"), embed=embed)
```

Both halves are required — `semantic=True` without an `embed` in the context is
a tool advertised as hybrid that quietly runs one arm.

## Mixing into a toolset of your own

`fs_tools()` returns the same tools as a plain list of pydantic-ai `Tool`
objects, taking the same `semantic` and `get_context` arguments.
`build_fs_toolset()` also forwards `**toolset_kwargs` (`max_retries`, `timeout`,
…) to `FunctionToolset`.

## Examples

- `examples/chat_agent.py` — the note-taking chat agent (`just agent`).
- `examples/semantic_search.py` — the hybrid setup end to end.
