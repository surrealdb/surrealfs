"""Framework-free tool definitions and dispatch.

Everything here is plain dicts and strings, so it drops into a hand-rolled
tool-calling loop against any provider:

    from surrealfs.integrations.json_tools import call_tool, tool_definitions
    from surrealfs.tools import ToolContext

    ctx = ToolContext(fs=SurrealFs(db))
    tools = tool_definitions()                      # -> Anthropic `tools=[...]`
    result = await call_tool(ctx, "ls", {"path": "/notes"})

See ``examples/anthropic_loop.py`` for a complete loop.
"""

from __future__ import annotations

from typing import Any, Literal

from ..errors import SurrealFsError
from ..tools import ToolContext, get_tool, select_tools

__all__ = ["call_tool", "tool_definitions"]

Flavor = Literal["anthropic", "openai"]


def tool_definitions(
    *, flavor: Flavor = "anthropic", semantic: bool = False
) -> list[dict[str, Any]]:
    """Tool definitions in the shape a provider's SDK expects.

    Args:
        flavor: ``"anthropic"`` produces ``{name, description, input_schema}``;
            ``"openai"`` produces ``{type: "function", function: {...}}``.
        semantic: Include ``search_semantic``. Only set this if the matching
            :class:`~surrealfs.tools.ToolContext` has an ``embed`` function.
    """
    specs = select_tools(semantic=semantic)
    if flavor == "anthropic":
        return [
            {
                "name": spec.name,
                "description": spec.description,
                "input_schema": spec.json_schema,
            }
            for spec in specs
        ]
    if flavor == "openai":
        return [
            {
                "type": "function",
                "function": {
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": spec.json_schema,
                },
            }
            for spec in specs
        ]
    raise ValueError(f"Unknown flavor {flavor!r}; expected 'anthropic' or 'openai'")


async def call_tool(ctx: ToolContext, name: str, arguments: dict[str, Any]) -> str:
    """Run a tool by name and return its result as text.

    Errors are returned as text rather than raised, since a tool-calling loop
    needs to hand the model something it can recover from. Genuine bugs (a
    dropped connection, a schema mismatch) still propagate.
    """
    try:
        spec = get_tool(name)
    except KeyError as exc:
        return f"Error: {exc}"
    try:
        return await spec(ctx, arguments)
    except SurrealFsError as exc:
        return f"Error: {exc}"
    except (ValueError, TypeError) as exc:
        # Argument validation failures — the model can fix these itself.
        return f"Error: invalid arguments for {name}: {exc}"
