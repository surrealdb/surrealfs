"""SurrealFS in a raw Anthropic tool-use loop -- no agent framework.

This is the whole integration: `tool_definitions()` for the request, and
`call_tool()` to run whatever the model asks for.

    just db
    export ANTHROPIC_API_KEY=...
    uv run python examples/anthropic_loop.py "Make me a packing list for Lisbon"
"""

from __future__ import annotations

import asyncio
import os
import sys

from anthropic import AsyncAnthropic
from surrealdb import AsyncSurreal

from surrealfs import SurrealFs, apply_schema
from surrealfs.integrations.json_tools import call_tool, tool_definitions
from surrealfs.tools import ToolContext

# No provider prefix here: this talks to the Anthropic SDK directly, which takes
# the bare model ID. (The pydantic-ai example needs an `anthropic:` prefix.)
MODEL = "claude-opus-5"
MAX_TOKENS = 16_000
MAX_TURNS = 20


async def run(prompt: str) -> str:
    db = AsyncSurreal(os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"))
    await db.signin({"username": "root", "password": "root"})
    await db.use("surrealfs", "demo")
    await apply_schema(db)

    ctx = ToolContext(fs=SurrealFs(db))
    client = AsyncAnthropic()
    tools = tool_definitions(flavor="anthropic")
    messages: list[dict] = [{"role": "user", "content": prompt}]

    try:
        for _ in range(MAX_TURNS):
            response = await client.messages.create(
                model=MODEL, max_tokens=MAX_TOKENS, tools=tools, messages=messages
            )
            messages.append({"role": "assistant", "content": response.content})

            tool_uses = [b for b in response.content if b.type == "tool_use"]
            if not tool_uses:
                return "".join(b.text for b in response.content if b.type == "text")

            results = []
            for block in tool_uses:
                print(f"  -> {block.name}({block.input})", file=sys.stderr)
                results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": await call_tool(ctx, block.name, block.input),
                    }
                )
            messages.append({"role": "user", "content": results})

        return f"Stopped after {MAX_TURNS} turns without a final answer."
    finally:
        await db.close()


if __name__ == "__main__":
    question = " ".join(sys.argv[1:]) or "List everything in my filesystem."
    print(asyncio.run(run(question)))
