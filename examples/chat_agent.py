"""A note-taking agent with a web chat UI, backed by SurrealFS.

    just db          # start SurrealDB in one terminal
    just agent       # then run this

Needs the demo extra and an Anthropic key:

    uv sync --extra demo
    export ANTHROPIC_API_KEY=...
"""

from __future__ import annotations

import asyncio
import os

import uvicorn
from pydantic_ai import Agent
from pydantic_ai.capabilities import WebFetch, WebSearch
from surrealdb import AsyncSurreal

from surrealfs import SurrealFs, apply_schema
from surrealfs.integrations.pydantic_ai import build_fs_toolset
from surrealfs.tools import ToolContext

INSTRUCTIONS = """\
You organise the user's thoughts, conversations, and notes into a well-structured
file system. You have a persistent filesystem; treat it as your memory.

Conventions to follow:
- Your working notes live in /notes.md. It may not exist yet, so start by
  listing / to see what is there rather than reading it blind, and create it
  once you have something worth keeping.
- Record anything you learn about the user's preferences under /preferences/.
- Give each project or task a folder under /projects/<project_name>/, with the
  notes and the current to-do list inside it.

Before answering, consider whether anything in this conversation belongs in a
file. Prefer `edit` over `write_file` when changing part of an existing file, and
search before you create — the note you want may already exist.
"""


def build_chat_agent(
    model: str = "anthropic:claude-opus-5", enable_web_tools: bool = True
) -> Agent[ToolContext, str]:
    """Build the agent.

    Note the ``anthropic:`` prefix on the model name: pydantic-ai validates bare
    strings against its own list of known models, so a plain ``claude-opus-5``
    raises ``UserError: Unknown model``. The prefix selects the provider
    explicitly and passes the ID straight through.
    """
    return Agent(
        model,
        toolsets=[build_fs_toolset()],
        instructions=INSTRUCTIONS,
        deps_type=ToolContext,
        # Server-side tools are `capabilities` in pydantic-ai 2.x (`builtin_tools`
        # from 1.x is gone). Pass the *capability* wrappers from
        # pydantic_ai.capabilities, not the WebSearchTool/WebFetchTool objects:
        # `capabilities` also accepts plain callables, so a raw tool object is
        # taken for one and invoked, failing at request time with
        # "'WebSearchTool' object is not callable".
        capabilities=[WebSearch(), WebFetch()] if enable_web_tools else [],
    )


async def connect() -> AsyncSurreal:
    """Connect, authenticate, and make sure the schema is in place."""
    db = AsyncSurreal(os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"))
    await db.signin(
        {
            "username": os.environ.get("SURREALDB_USER", "root"),
            "password": os.environ.get("SURREALDB_PASS", "root"),
        }
    )
    await db.use(
        os.environ.get("SURREALDB_NAMESPACE", "surrealfs"),
        os.environ.get("SURREALDB_DATABASE", "demo"),
    )
    await apply_schema(db)
    return db


async def serve(host: str = "127.0.0.1", port: int = 7932) -> None:
    """Connect and serve the chat UI on a single event loop.

    The connection has to be opened on the same loop that will serve requests.
    `uvicorn.run()` creates its own loop, so connecting via a separate
    `asyncio.run(connect())` first leaves the WebSocket bound to a loop that is
    already closed, and the first message fails with "Future attached to a
    different loop" — surfaced in the UI only as an opaque TaskGroup error.
    Driving `uvicorn.Server` ourselves keeps everything on one loop.
    """
    db = await connect()
    agent = build_chat_agent()
    app = agent.to_web(deps=ToolContext(fs=SurrealFs(db)))
    try:
        await uvicorn.Server(uvicorn.Config(app, host=host, port=port)).serve()
    finally:
        await db.close()


def main() -> None:
    try:
        import logfire
    except ImportError:
        print("logfire is missing. Install the demo extra: uv sync --extra demo")
        raise

    logfire.configure(send_to_logfire="if-token-present")
    logfire.instrument_pydantic_ai()
    logfire.instrument_anthropic()

    asyncio.run(serve())


if __name__ == "__main__":
    main()
