"""A note-taking agent with pydantic-ai's own web chat UI, backed by SurrealFS.

    just db          # start SurrealDB in one terminal
    just agent       # then run this

Needs the demo extra and an Anthropic key:

    uv sync --extra demo
    export ANTHROPIC_API_KEY=...

The agent itself is `surrealfs.browser.agent`, the same one the file browser's
chat panel drives -- run `surrealfs-browser` instead to get it alongside a view
of the filesystem it is writing to.
"""

from __future__ import annotations

import asyncio

import uvicorn

from surrealfs import SurrealFs, apply_schema
from surrealfs.browser.agent import build_chat_agent
from surrealfs.embed import make_embedder
from surrealfs.integrations._connect import connect
from surrealfs.tools import ToolContext


async def serve(host: str = "127.0.0.1", port: int = 7932) -> None:
    """Connect and serve the chat UI on a single event loop.

    The connection has to be opened on the same loop that will serve requests.
    `uvicorn.run()` creates its own loop, so connecting via a separate
    `asyncio.run(connect())` first leaves the WebSocket bound to a loop that is
    already closed, and the first message fails with "Future attached to a
    different loop" — surfaced in the UI only as an opaque TaskGroup error.
    Driving `uvicorn.Server` ourselves keeps everything on one loop.
    """
    embed = make_embedder()
    db = await connect()
    await apply_schema(db)
    agent = build_chat_agent()
    app = agent.to_web(deps=ToolContext(fs=SurrealFs(db, user="demo"), embed=embed))
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
