"""A Hermes plugin that gives the agent SurrealFS as its filesystem.

See https://hermes-agent.nousresearch.com/docs/developer-guide/plugins.

Installing the package registers the plugin through an entry point. Hermes runs
from its own virtualenv, so it has to be installed *there*::

    uv pip install --python ~/.hermes/hermes-agent/venv/bin/python surrealfs
    hermes plugins enable surrealfs
    hermes chat --toolsets surrealfs

To run from a checkout, symlink this directory into Hermes' plugin folder — the
package still has to be importable by Hermes' interpreter::

    ln -s "$PWD/surrealfs/integrations/hermes" ~/.hermes/plugins/surrealfs

The tools arrive as ``surrealfs_ls``, ``surrealfs_cat``, ``surrealfs_write_file``
and so on — Hermes has one flat tool namespace and its own file tools, which read
and write the local disk, already sit in it.

Connection details come from the same ``SURREALDB_*`` environment variables the
rest of the project uses, defaulting to a local server. Set
``SURREALFS_SEMANTIC=1`` to let `surrealfs_search` fuse in match-by-meaning results;
that needs ``OPENAI_API_KEY``, the ``embed`` extra, and the indexer on a schedule,
since nothing embeds a file as it is written. Hermes has no plugin-declared daemon,
so the least setup is one of its own script-only cron jobs::

    hermes cron create 'every 5m' --no-agent --script surrealfs-embed.sh

See ``docs/hermes-indexer.md`` for the script.

A skill in ``skills/notes/`` ships with the plugin, teaching the agent to keep its
notes and memories here and how to organise them. Hermes does not advertise plugin
skills, so a ``pre_llm_call`` hook names it on the first turn of each session —
see :func:`_on_pre_llm_call`.

Nothing here imports Hermes: the plugin contract is a ``register`` function and a
manifest, so there is no dependency to install.
"""

from __future__ import annotations

import json
import os
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any

from surrealfs import SurrealFs
from surrealfs.embed import make_embedder
from surrealfs.integrations._connect import agent_user, connected
from surrealfs.integrations.json_tools import call_tool
from surrealfs.tools import TOOLS, ToolContext, select_tools

__all__ = ["PREFIX", "SKILL", "TOOLSET", "register"]

TOOLSET = "surrealfs"

# Hermes registers tools in one flat namespace and *silently drops* a name that
# is already claimed by another toolset, so `write_file` would lose to the
# built-in disk one. A prefix keeps all fourteen, and tells the model which
# filesystem it is addressing — Hermes' own `read_file`/`write_file` are the
# real disk, `surrealfs_*` is the shared one in SurrealDB. Spelled out rather
# than abbreviated so the name in the tool matches the name in its description.
PREFIX = "surrealfs_"

SKILL = Path(__file__).parent / "skills" / "notes" / "SKILL.md"

_SKILL_DESCRIPTION = (
    "Keep notes, memories, plans, to-dos, and documents in SurrealFS — a "
    "persistent filesystem the user can read too."
)

# What the agent is told on its first turn. Hermes appends this to that turn's
# user message and never persists it, so the system prompt — and the prompt cache
# prefix built on it — stays byte-stable.
_FIRST_TURN_NUDGE = (
    "You have SurrealFS: a filesystem in SurrealDB, reached through the "
    "`surrealfs_*` tools, that persists across sessions and that the user can read "
    "too. Notes, memories, plans, to-dos, and any document worth keeping belong "
    "there rather than in a reply that scrolls away. Run "
    "skill_view('surrealfs:notes') for the folder conventions before writing."
)


def register(ctx: Any) -> None:
    """Register the SurrealFS tools, the bundled skill, and the first-turn nudge.

    ``ctx`` is Hermes' plugin context.
    """
    semantic = _semantic()
    for spec in select_tools(semantic=semantic):
        ctx.register_tool(
            name=PREFIX + spec.name,
            toolset=TOOLSET,
            schema={
                "name": PREFIX + spec.name,
                "description": _describe(spec.description),
                "parameters": spec.json_schema,
            },
            handler=_handler(spec.name, semantic=semantic),
            # Hermes bridges async handlers itself, via `model_tools._run_async`.
            # Doing it here instead would break under the gateways: they dispatch
            # tools from inside a running loop, where `asyncio.run` raises.
            is_async=True,
        )

    # After the tools: if the skill file is missing from a build, this raises
    # FileNotFoundError, and the tools are already registered by then.
    ctx.register_skill("notes", SKILL, description=_SKILL_DESCRIPTION)
    ctx.register_hook("pre_llm_call", _on_pre_llm_call)


def _on_pre_llm_call(
    is_first_turn: bool = False, **kwargs: Any
) -> dict[str, str] | None:
    """Point the agent at the bundled skill, once per session.

    Hermes does not advertise plugin skills: they are absent from the system
    prompt's ``<available_skills>`` index and from ``skills_list()``, and resolve
    only through the exact qualified name. Without this the skill would be
    unreachable — nothing would ever tell the agent it exists.
    """
    if not is_first_turn:
        return None
    return {"context": _FIRST_TURN_NUDGE}


def _describe(description: str) -> str:
    """Point the description's cross-references at the names the model sees.

    The markdown in ``surrealfs/tools/docs`` names sibling tools — "prefer
    `cat`", "use `edit`" — and under a prefix those tools do not exist.
    """
    for spec in TOOLS:
        description = description.replace(f"`{spec.name}`", f"`{PREFIX}{spec.name}`")
    return description


def _semantic() -> bool:
    return os.environ.get("SURREALFS_SEMANTIC", "").strip().lower() in {
        "1",
        "true",
        "yes",
    }


def _handler(name: str, *, semantic: bool) -> Callable[..., Awaitable[str]]:
    """Wrap one tool as a Hermes handler: JSON out, never raises."""

    async def run(args: dict[str, Any], **kwargs: Any) -> str:
        try:
            result = await _dispatch(name, args, semantic=semantic)
        except Exception as exc:
            # A Hermes handler must not raise. `call_tool` already turns the
            # failures a model can fix into text, so whatever lands here is a
            # dead connection or a schema mismatch — report it and move on.
            return json.dumps({"error": f"{type(exc).__name__}: {exc}"})
        return json.dumps({"result": result})

    run.__name__ = PREFIX + name
    run.__doc__ = f"Hermes handler for the SurrealFS `{name}` tool."
    return run


async def _dispatch(name: str, args: dict[str, Any], *, semantic: bool) -> str:
    """Connect, run one tool, and hand back its text."""
    async with connected() as db:
        # The embedder is built per call, not cached: the OpenAI client binds its
        # connection pool to the running loop, and this one closes with the call.
        embed = make_embedder() if semantic else None
        ctx = ToolContext(fs=SurrealFs(db, user=agent_user()), embed=embed)
        return await call_tool(ctx, name, args, semantic=semantic)
