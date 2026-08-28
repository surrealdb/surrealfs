# SurrealFS as raw JSON tool schemas

No framework, just dicts and strings — drops into a hand-rolled tool-use loop
against any provider.

```python
from surrealfs.integrations.json_tools import call_tool, tool_definitions
from surrealfs.tools import ToolContext

ctx = ToolContext(fs=SurrealFs(db, user="alice"))
tools = tool_definitions(flavor="anthropic")  # or flavor="openai"

# inside your tool-use loop:
result = await call_tool(ctx, block.name, block.input)
```

`flavor="anthropic"` produces `{name, description, input_schema}`;
`flavor="openai"` produces `{type: "function", function: {...}}`. Same tools
either way.

`call_tool` returns errors as text rather than raising them, since a tool-use
loop needs to hand the model something it can recover from. Genuine bugs — a
dropped connection, a schema mismatch — still propagate.

## Hybrid search

Pass `semantic=True` to **both** calls, and put an `embed` function in the
context:

```python
tools = tool_definitions(semantic=True)
result = await call_tool(ctx, name, args, semantic=True)
```

Pass it to only one and `search` is advertised as hybrid while quietly running
full-text.

## Example

`examples/anthropic_loop.py` — a complete loop in ~40 lines (`just loop "…"`).
