"""The SurrealFS tool registry.

One list of :class:`ToolSpec`s drives every integration. A tool's name, argument
schema, description, and implementation live together here, so adding a tool
makes it appear in the pydantic-ai toolset, the raw JSON definitions, and the
Hermes plugin at once.

Descriptions are authored as markdown in ``surrealfs/tools/docs`` rather than in
docstrings — they are prompt text, and worth editing as prose.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from functools import cached_property, partial
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from . import args as _args
from . import handlers as _handlers
from .handlers import ToolContext

__all__ = ["TOOLS", "ToolContext", "ToolSpec", "get_tool", "select_tools"]

_DOCS_DIR = Path(__file__).resolve().parent / "docs"

Handler = Callable[[ToolContext, Any], Awaitable[str]]


@dataclass(frozen=True)
class ToolSpec:
    """Everything an integration needs to expose one tool."""

    name: str
    args_model: type[BaseModel]
    handler: Handler

    @cached_property
    def description(self) -> str:
        """Prompt text, read from ``docs/<kebab-case-name>.md``."""
        path = _DOCS_DIR / f"{self.name.replace('_', '-')}.md"
        return path.read_text(encoding="utf-8").strip()

    @cached_property
    def json_schema(self) -> dict[str, Any]:
        """JSON Schema for this tool's arguments."""
        return self.args_model.model_json_schema()

    async def __call__(self, ctx: ToolContext, arguments: dict[str, Any]) -> str:
        """Validate raw arguments and run the tool."""
        return await self.handler(ctx, self.args_model.model_validate(arguments))


TOOLS: tuple[ToolSpec, ...] = (
    ToolSpec("ls", _args.LsArgs, _handlers.ls),
    ToolSpec("glob", _args.GlobArgs, _handlers.glob),
    ToolSpec("cat", _args.CatArgs, _handlers.cat),
    ToolSpec("read_bytes", _args.ReadBytesArgs, _handlers.read_bytes),
    ToolSpec("tail", _args.TailArgs, _handlers.tail),
    ToolSpec("write_file", _args.WriteFileArgs, _handlers.write_file),
    ToolSpec("write_bytes", _args.WriteBytesArgs, _handlers.write_bytes),
    ToolSpec("edit", _args.EditArgs, _handlers.edit),
    ToolSpec("touch", _args.TouchArgs, _handlers.touch),
    ToolSpec("mkdir", _args.MkdirArgs, _handlers.mkdir),
    ToolSpec("cp", _args.CpArgs, _handlers.cp),
    ToolSpec("mv", _args.MvArgs, _handlers.mv),
    ToolSpec("rm", _args.RmArgs, _handlers.rm),
    ToolSpec("chmod", _args.ChmodArgs, _handlers.chmod),
    ToolSpec("search", _args.SearchArgs, _handlers.search),
)

# Same name, so the same `docs/search.md` describes both -- the description is
# written to hold either way, and one tool the model always reaches for beats two
# it has to choose between.
_HYBRID_SEARCH = ToolSpec(
    "search", _args.SearchArgs, partial(_handlers.search, semantic=True)
)


def select_tools(*, semantic: bool = False) -> tuple[ToolSpec, ...]:
    """The tools to expose.

    ``semantic=True`` lets `search` fuse vector results into its full-text
    ranking. That needs a ``ToolContext.embed``; without one the tool is still
    offered and still works, full-text only.
    """
    if not semantic:
        return TOOLS
    return tuple(_HYBRID_SEARCH if spec.name == "search" else spec for spec in TOOLS)


def get_tool(name: str, *, semantic: bool = False) -> ToolSpec:
    """Look up a tool by name.

    Pass the same ``semantic`` you passed when listing the tools: it selects
    which `search` the model gets, and advertising the hybrid one while
    dispatching to the full-text one is a silent downgrade.
    """
    specs = select_tools(semantic=semantic)
    for spec in specs:
        if spec.name == name:
            return spec
    known = ", ".join(spec.name for spec in specs)
    raise KeyError(f"Unknown tool {name!r}. Available tools: {known}")
