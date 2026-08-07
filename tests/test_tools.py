"""The tool registry and every integration surface."""

from __future__ import annotations

import base64
import json
from pathlib import Path

import pytest

from surrealfs.integrations.json_tools import call_tool, tool_definitions
from surrealfs.tools import TOOLS, ToolContext, get_tool, select_tools


class _FakeHermesCtx:
    """Just enough of Hermes' plugin context to capture what `register` wires up."""

    def __init__(self) -> None:
        self.schemas: list[dict] = []
        self.handlers: dict = {}
        self.toolsets: set[str] = set()
        self.is_async: set[bool] = set()
        self.skills: dict = {}
        self.hooks: dict = {}

    def register_tool(self, *, name, toolset, schema, handler, **kwargs) -> None:
        self.schemas.append(schema)
        self.handlers[name] = handler
        self.toolsets.add(toolset)
        self.is_async.add(kwargs.get("is_async", False))

    def register_skill(self, name, path, description="") -> None:
        self.skills[name] = (path, description)

    def register_hook(self, event, callback) -> None:
        self.hooks[event] = callback


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


def test_search_is_one_tool_either_way():
    """`semantic` changes what `search` does, not which tools exist."""
    default = {spec.name for spec in select_tools()}
    hybrid = {spec.name for spec in select_tools(semantic=True)}
    assert default == hybrid
    assert "search" in default
    assert not {"search_text", "search_semantic"} & default


def test_surfaces_expose_the_same_tools():
    """The guard against the integrations drifting apart."""
    registry = [spec.name for spec in select_tools()]
    anthropic = [t["name"] for t in tool_definitions()]
    openai = [t["function"]["name"] for t in tool_definitions(flavor="openai")]
    assert registry == anthropic == openai

    from surrealfs.integrations import hermes

    hermes_ctx = _FakeHermesCtx()
    hermes.register(hermes_ctx)
    prefixed = [hermes.PREFIX + name for name in registry]
    assert [schema["name"] for schema in hermes_ctx.schemas] == prefixed
    assert hermes_ctx.toolsets == {hermes.TOOLSET}

    # Hermes names the argument schema `parameters`, unlike either JSON flavour.
    ls = next(s for s in hermes_ctx.schemas if s["name"] == "surrealfs_ls")
    assert ls["parameters"] == get_tool("ls").json_schema

    # `provides_tools` is hand-written, so it is the one place that can drift.
    manifest = (Path(hermes.__file__).parent / "plugin.yaml").read_text(
        encoding="utf-8"
    )
    for name in prefixed:
        assert f"- {name}\n" in manifest, f"plugin.yaml omits {name}"

    pytest.importorskip("pydantic_ai")
    from surrealfs.integrations.pydantic_ai import fs_tools

    assert [t.name for t in fs_tools()] == registry


def test_hermes_descriptions_point_at_the_prefixed_names():
    """The shared markdown names sibling tools; unprefixed they do not exist."""
    from surrealfs.integrations import hermes

    assert "`read_bytes`" in get_tool("cat").description
    ctx = _FakeHermesCtx()
    hermes.register(ctx)
    cat = next(s for s in ctx.schemas if s["name"] == "surrealfs_cat")
    assert "`surrealfs_read_bytes`" in cat["description"]
    stripped = cat["description"].replace("`surrealfs_read_bytes`", "")
    assert "`read_bytes`" not in stripped


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
    assert "/notes/deep/a.md" in await call_tool(ctx, "search", {"query": "bye"})
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


async def test_hybrid_search_without_an_embedder_degrades_quietly(fs):
    """No embedder is not an error: the model cannot fix that by retrying."""
    await fs.write_text("/target.md", "the answer")
    ctx = ToolContext(fs=fs)  # semantic=True, but nothing to embed with
    result = await call_tool(ctx, "search", {"query": "answer"}, semantic=True)
    assert not result.startswith("Error:")
    assert "/target.md" in result


async def test_hybrid_search_finds_what_words_cannot(fs, db):
    async def embed(text: str) -> list[float]:
        vector = [0.0] * 1536
        vector[3] = 1.0
        return vector

    entry = await fs.write_text("/target.md", "invoicing and getting paid")
    await db.query(
        "UPDATE $id SET embedding = $v", {"id": entry.id, "v": await embed("")}
    )
    ctx = ToolContext(fs=fs, embed=embed)
    args = {"query": "remuneration"}  # shares no words with the file
    assert "/target.md" not in await call_tool(ctx, "search", args)
    assert "/target.md" in await call_tool(ctx, "search", args, semantic=True)


def test_hermes_bundles_the_notes_skill():
    """The skill has to be on disk where `register` says it is, docs and all."""
    from surrealfs.integrations import hermes

    ctx = _FakeHermesCtx()
    hermes.register(ctx)

    path, description = ctx.skills["notes"]
    assert path == hermes.SKILL
    assert description, "Hermes shows this when a skill name misses"

    text = path.read_text(encoding="utf-8")
    assert text.startswith("---\n")
    assert "\nname: notes\n" in text
    assert "\ndescription: " in text
    # The conventions the agent is meant to follow.
    for folder in ("/home/hermes/", "/preferences/", "/projects/"):
        assert folder in text

    # Hermes logs a prompt-injection warning on every load if any of these appear
    # (`_INJECTION_PATTERNS` in its tools/skills_tool.py).
    for phrase in ("ignore previous instructions", "you are now", "new instructions:"):
        assert phrase not in text.lower()


def test_hermes_nudges_once_per_session():
    """Plugin skills are advertised nowhere, so the hook is the only way in."""
    from surrealfs.integrations import hermes

    ctx = _FakeHermesCtx()
    hermes.register(ctx)
    nudge = ctx.hooks["pre_llm_call"]

    first = nudge(session_id="s1", user_message="hi", is_first_turn=True, model="m")
    assert "surrealfs:notes" in first["context"]
    assert "surrealfs_" in first["context"]

    # Every later turn stays out of the prompt.
    assert nudge(session_id="s1", user_message="and again", is_first_turn=False) is None
    assert nudge() is None, "a caller omitting is_first_turn must not be nudged"


async def test_hermes_handlers_round_trip(surreal_url, monkeypatch):
    """Exercise the Hermes handlers, including the connection they own."""
    from surrealfs.integrations import _connect, hermes

    monkeypatch.setenv("SURREALDB_URL", surreal_url)
    monkeypatch.setenv("SURREALDB_NAMESPACE", "hermes_plugin_test")
    monkeypatch.setenv("SURREALDB_DATABASE", "test")
    monkeypatch.setattr(_connect, "_schema_applied", False)

    ctx = _FakeHermesCtx()
    hermes.register(ctx)
    assert ctx.is_async == {True}, "Hermes must await these, not call them"

    write = ctx.handlers["surrealfs_write_file"]
    written = json.loads(await write({"path": "/a.md", "content": "hi"}))
    assert "/a.md" in written["result"]
    read = json.loads(await ctx.handlers["surrealfs_cat"]({"path": "/a.md"}))
    assert read["result"] == "hi"

    # Failures the model can fix stay in `result` as text, same as any other
    # dispatch; Hermes only ever sees JSON.
    missing = json.loads(await ctx.handlers["surrealfs_cat"]({"path": "/missing.md"}))
    assert missing["result"].startswith("Error:")

    # And a dead connection is reported rather than raised.
    monkeypatch.setenv("SURREALDB_URL", "ws://127.0.0.1:1/rpc")
    assert "error" in json.loads(await ctx.handlers["surrealfs_ls"]({"path": "/"}))


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
