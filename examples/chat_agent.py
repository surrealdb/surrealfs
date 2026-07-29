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
from pydantic_ai import Agent, ModelSettings, WebFetchTool, WebSearchTool
from surrealdb import AsyncSurreal

from surrealfs import SurrealFs, apply_schema
from surrealfs.integrations.pydantic_ai import build_fs_toolset
from surrealfs.tools import ToolContext

INSTRUCTIONS = """\
You organise the user's thoughts, conversations, and notes into a well-structured
file system. You have a persistent filesystem; treat it as your memory.

Conventions to follow:
- Keep your working notes in /notes.md. Read it at the start of every
  conversation, and update it when you learn something worth keeping.
- Record anything you learn about the user's preferences under /preferences/.
- Give each project or task a folder under /projects/<project_name>/, with the
  notes and the current to-do list inside it.

Before answering, consider whether anything in this conversation belongs in a
file. Prefer `edit` over `write_file` when changing part of an existing file, and
search before you create — the note you want may already exist.
"""


def build_chat_agent(
    model: str = "claude-sonnet-5", enable_web_tools: bool = True
) -> Agent[ToolContext, str]:
    return Agent(
        model,
        toolsets=[build_fs_toolset()],
        instructions=INSTRUCTIONS,
        deps_type=ToolContext,
        model_settings=ModelSettings(
            extra_headers={"anthropic-beta": "context-management-2025-06-27"}
        ),
        builtin_tools=[WebFetchTool(), WebSearchTool()] if enable_web_tools else [],
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


def main() -> None:
    try:
        import logfire
    except ImportError:
        print("logfire is missing. Install the demo extra: uv sync --extra demo")
        raise

    logfire.configure(send_to_logfire="if-token-present")
    logfire.instrument_pydantic_ai()
    logfire.instrument_anthropic()

    db = asyncio.run(connect())
    agent = build_chat_agent()
    app = agent.to_web(deps=ToolContext(fs=SurrealFs(db)))
    uvicorn.run(app, host="127.0.0.1", port=7932)


if __name__ == "__main__":
    main()
