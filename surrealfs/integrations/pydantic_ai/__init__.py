"""A pydantic-ai ``FunctionToolset`` over the SurrealFS tools.

Requires the ``pydantic-ai`` extra:

    pip install "surrealfs[pydantic-ai]"

Usage — the agent's deps carry the filesystem:

    from pydantic_ai import Agent
    from surrealfs import SurrealFs
    from surrealfs.integrations.pydantic_ai import build_fs_toolset
    from surrealfs.tools import ToolContext

    agent = Agent("claude-sonnet-5", deps_type=ToolContext,
                  toolsets=[build_fs_toolset()])
    await agent.run("Summarise my notes", deps=ToolContext(fs=SurrealFs(db)))
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any, TypeVar

from pydantic import ValidationError
from pydantic_ai import ModelRetry, RunContext, Tool
from pydantic_ai.toolsets import FunctionToolset

from ...errors import QueryError, SurrealFsError
from ...tools import ToolContext, ToolSpec, select_tools

__all__ = ["build_fs_toolset", "fs_tools"]

DepsT = TypeVar("DepsT")


def _default_context(deps: Any) -> ToolContext:
    if isinstance(deps, ToolContext):
        return deps
    context = getattr(deps, "fs_context", None)
    if isinstance(context, ToolContext):
        return context
    raise TypeError(
        "Agent deps must be a surrealfs.tools.ToolContext, or expose one as "
        "`.fs_context`. Alternatively pass get_context= to build_fs_toolset()."
    )


def _as_tool(spec: ToolSpec, get_context: Callable[[Any], ToolContext]) -> Tool[Any]:
    async def run(ctx: RunContext[Any], **kwargs: Any) -> str:
        try:
            return await spec(get_context(ctx.deps), kwargs)
        except QueryError:
            # A malformed query or a dead connection is our bug, not the
            # model's — let it surface instead of prompting a pointless retry.
            raise
        except (SurrealFsError, ValidationError, ValueError) as exc:
            # Wrong path, missing text, bad arguments: the model can correct
            # these itself, so hand the message back and let it try again.
            raise ModelRetry(str(exc)) from exc

    return Tool.from_schema(
        run,
        name=spec.name,
        description=spec.description,
        json_schema=spec.json_schema,
        takes_ctx=True,
    )


def fs_tools(
    *,
    semantic: bool = False,
    get_context: Callable[[Any], ToolContext] = _default_context,
) -> list[Tool[Any]]:
    """The SurrealFS tools as a list of pydantic-ai ``Tool`` objects.

    Use this to mix the filesystem tools into a toolset of your own; use
    :func:`build_fs_toolset` if you just want them on their own.
    """
    return [_as_tool(spec, get_context) for spec in select_tools(semantic=semantic)]


def build_fs_toolset(
    *,
    semantic: bool = False,
    get_context: Callable[[Any], ToolContext] = _default_context,
    **toolset_kwargs: Any,
) -> FunctionToolset[Any]:
    """Build a ``FunctionToolset`` exposing the filesystem to an agent.

    Args:
        semantic: Let ``search`` fuse match-by-meaning results into its ranking.
            Requires the run's :class:`~surrealfs.tools.ToolContext` to carry an
            ``embed`` function, so it is off by default.
        get_context: Extract a ``ToolContext`` from the agent's deps. The
            default accepts a ``ToolContext`` directly, or any deps object with
            an ``.fs_context`` attribute.
        **toolset_kwargs: Passed through to ``FunctionToolset`` (``max_retries``,
            ``timeout``, and so on).
    """
    return FunctionToolset(
        fs_tools(semantic=semantic, get_context=get_context), **toolset_kwargs
    )
