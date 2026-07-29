"""The tool registry and both integration surfaces."""

from __future__ import annotations

import base64

import pytest

from surrealfs.integrations.json_tools import call_tool, tool_definitions
from surrealfs.tools import TOOLS, ToolContext, get_tool, select_tools


def test_every_tool_has_a_description_and_schema():
    for spec in TOOLS:
        assert spec.description, f"{spec.name} has no docs/*.md description"
        assert len(spec.description) > 40, f"{spec.name} description is too thin"
        schema = spec.json_schema
        assert schema["type"] == "object"
        assert "properties" in schema


def test_tool_names_are_unique():
    names = [spec.name for spec in TOOLS]
    assert len(names) == len(set(names))


def test_semantic_search_is_opt_in():
    """A tool that always errors is worse than one that is not offered."""
    default = {spec.name for spec in select_tools()}
    assert "search_semantic" not in default
    assert "search_semantic" in {spec.name for spec in select_tools(semantic=True)}


def test_surfaces_expose_the_same_tools():
    """The guard against the two integrations drifting apart."""
    registry = [spec.name for spec in select_tools()]
    anthropic = [t["name"] for t in tool_definitions()]
    openai = [t["function"]["name"] for t in tool_definitions(flavor="openai")]
    assert registry == anthropic == openai

    pytest.importorskip("pydantic_ai")
    from surrealfs.integrations.pydantic_ai import fs_tools

    assert [t.name for t in fs_tools()] == registry


def test_definitions_carry_the_markdown_descriptions():
    ls = next(t for t in tool_definitions() if t["name"] == "ls")
    assert ls["description"] == get_tool("ls").description
    assert ls["input_schema"] == get_tool("ls").json_schema


def test_unknown_flavor_is_rejected():
    with pytest.raises(ValueError):
        tool_definitions(flavor="bogus")


def test_get_tool_reports_available_names():
    with pytest.raises(KeyError, match="write_file"):
        get_tool("nope")


async def test_tools_round_trip_through_dispatch(ctx):
    assert "Created folder /notes" in await call_tool(
        ctx, "mkdir", {"path": "/notes/deep", "parents": True}
    )
    await call_tool(ctx, "write_file", {"path": "/notes/deep/a.md", "content": "hi"})
    assert await call_tool(ctx, "cat", {"path": "/notes/deep/a.md"}) == "hi"

    listing = await call_tool(ctx, "ls", {"path": "/", "recursive": True})
    assert "/notes/" in listing and "/notes/deep/a.md" in listing

    assert "+bye" in await call_tool(
        ctx, "edit", {"path": "/notes/deep/a.md", "old": "hi", "new": "bye"}
    )
    assert "/notes/deep/a.md" in await call_tool(
        ctx, "glob", {"pattern": "/notes/**/*.md"}
    )
    assert "/notes/deep/a.md" in await call_tool(ctx, "search_text", {"query": "bye"})
    assert "Moved" in await call_tool(ctx, "mv", {"src": "/notes", "dst": "/n2"})
    assert "Deleted 3 items" in await call_tool(
        ctx, "rm", {"path": "/n2", "recursive": True}
    )


async def test_binary_tools_use_base64(ctx):
    payload = base64.b64encode(b"\x89PNG\x00\xff").decode()
    await call_tool(
        ctx,
        "write_bytes",
        {"path": "/x.png", "data_base64": payload, "content_type": "image/png"},
    )
    assert await call_tool(ctx, "read_bytes", {"path": "/x.png"}) == payload


async def test_invalid_base64_is_reported_not_raised(ctx):
    result = await call_tool(
        ctx, "write_bytes", {"path": "/x.png", "data_base64": "not base64!"}
    )
    assert result.startswith("Error:")
    assert "base64" in result


async def test_dispatch_returns_recoverable_errors_as_text(ctx):
    """A tool-calling loop needs something to hand back to the model."""
    assert (await call_tool(ctx, "cat", {"path": "/missing.md"})).startswith("Error:")
    assert (await call_tool(ctx, "ls", {"path": "/", "bogus": 1})).startswith("Error:")
    assert (await call_tool(ctx, "nosuchtool", {})).startswith("Error:")


async def test_semantic_tool_without_an_embedder_reports_clearly(ctx):
    result = await call_tool(ctx, "search_semantic", {"query": "anything"})
    assert result.startswith("Error:")
    assert "not configured" in result


async def test_semantic_tool_with_an_embedder(fs, db):
    async def embed(text: str) -> list[float]:
        vector = [0.0] * 1536
        vector[3] = 1.0
        return vector

    entry = await fs.write_text("/target.md", "the answer")
    await db.query(
        "UPDATE $id SET embedding = $v", {"id": entry.id, "v": await embed("")}
    )
    ctx = ToolContext(fs=fs, embed=embed)
    assert "/target.md" in await call_tool(ctx, "search_semantic", {"query": "answer"})


async def test_pydantic_ai_tools_run(ctx):
    """Exercise the pydantic-ai wrapper without standing up a model."""
    pytest.importorskip("pydantic_ai")
    from pydantic_ai import ModelRetry
    from pydantic_ai.tools import RunContext

    from surrealfs.integrations.pydantic_ai import fs_tools

    tools = {tool.name: tool for tool in fs_tools()}
    run_context = RunContext(deps=ctx, model=None, usage=None)

    await ctx.fs.write_text("/a.md", "hello")
    assert await tools["cat"].function(run_context, path="/a.md") == "hello"

    # Recoverable failures become ModelRetry so the agent tries again.
    with pytest.raises(ModelRetry):
        await tools["cat"].function(run_context, path="/missing.md")
