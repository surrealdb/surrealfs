"""The MCP server every client shares, and its Spectron mirror."""

from __future__ import annotations

import sys

from surrealfs.integrations import mcp
from surrealfs.tools import select_tools


def test_mcp_surface_matches_the_registry():
    """The drift guard: a tool added to the registry has to reach MCP too."""
    registry = [spec.name for spec in select_tools()]
    assert [spec["name"] for spec in mcp.tool_specs(recall=False)] == registry
    assert [spec["name"] for spec in mcp.tool_specs(recall=True)] == registry + [
        mcp.RECALL_TOOL
    ]

    for spec in mcp.tool_specs(recall=True):
        assert spec["description"].strip(), spec["name"]
        assert spec["schema"].get("type") == "object", spec["name"]

    # Unprefixed, unlike Hermes: an MCP client namespaces tools by server.
    assert not any(name.startswith("surrealfs_") for name in registry)


def test_recall_is_only_offered_once_spectron_is_configured(monkeypatch):
    """Spectron is optional, so the tool that needs it is too.

    A `brain_recall` whose every answer is "Spectron is not configured" costs the
    model a tool slot and a round trip to learn nothing it can act on.
    """
    # `_clean_surrealdb_env` in conftest has already stripped every SPECTRON_ var.
    assert mcp.RECALL_TOOL not in [spec["name"] for spec in mcp.tool_specs()]

    monkeypatch.setenv("SPECTRON_CONTEXT_ID", "ctx")
    monkeypatch.setenv("SPECTRON_API_KEY", "sp-key")
    assert mcp.RECALL_TOOL in [spec["name"] for spec in mcp.tool_specs()]

    # A key on its own is not a configuration, and neither is a context.
    monkeypatch.delenv("SPECTRON_API_KEY")
    assert mcp.RECALL_TOOL not in [spec["name"] for spec in mcp.tool_specs()]


async def test_a_configured_spectron_without_httpx_names_the_extra(ctx, monkeypatch):
    """The one failure mode the `spectron` extra introduces.

    httpx is no longer part of `surrealfs[mcp]`, so a key can be set on an install
    that cannot make an HTTP request. That has to name the missing extra rather
    than surface as a bare ModuleNotFoundError, and it must not be swallowed: a
    memory layer that silently files nothing is the worse failure.
    """
    monkeypatch.setenv("SPECTRON_CONTEXT_ID", "ctx")
    monkeypatch.setenv("SPECTRON_API_KEY", "sp-key")
    monkeypatch.setitem(sys.modules, "httpx", None)  # `import httpx` now raises

    recalled = await mcp.run_tool(
        ctx, mcp.RECALL_TOOL, {"query": "what is blocking us"}
    )
    assert "surrealfs[mcp,spectron]" in recalled

    written = await mcp.run_tool(
        ctx, "write_file", {"path": "/brain/note.md", "content": "kept"}
    )
    assert "surrealfs[mcp,spectron]" in written
    assert await ctx.fs.read_text("/brain/note.md") == "kept"


async def test_a_write_is_mirrored_and_a_read_is_not(ctx):
    sent: list[tuple[str, str]] = []

    async def fake(path: str, text: str) -> None:
        sent.append((path, text))

    result = await mcp.run_tool(
        ctx,
        "write_file",
        {"path": "/brain/acme/risks/okta.md", "content": "cert expires 2026-10-02"},
        mirror=fake,
    )
    assert "Error" not in result
    assert sent == [("/brain/acme/risks/okta.md", "cert expires 2026-10-02")]

    await mcp.run_tool(ctx, "cat", {"path": "/brain/acme/risks/okta.md"}, mirror=fake)
    assert len(sent) == 1, "reads must not touch Spectron"

    # An `edit` mirrors the file as it now stands, not the replacement text.
    await mcp.run_tool(
        ctx,
        "edit",
        {"path": "/brain/acme/risks/okta.md", "old": "2026-10-02", "new": "2026-11-30"},
        mirror=fake,
    )
    assert sent[-1] == ("/brain/acme/risks/okta.md", "cert expires 2026-11-30")


async def test_a_failed_write_is_not_mirrored(ctx):
    sent: list[str] = []

    async def fake(path: str, text: str) -> None:
        sent.append(path)

    result = await mcp.run_tool(
        ctx, "edit", {"path": "/nope.md", "old": "a", "new": "b"}, mirror=fake
    )
    assert result.startswith("Error:")
    assert sent == []


async def test_a_broken_mirror_keeps_the_write_and_says_so(ctx):
    async def broken(path: str, text: str) -> None:
        raise RuntimeError("spectron unreachable")

    result = await mcp.run_tool(
        ctx, "write_file", {"path": "/brain/note.md", "content": "kept"}, mirror=broken
    )
    assert "brain sync failed" in result
    assert await ctx.fs.read_text("/brain/note.md") == "kept"


async def test_the_mirror_root_confines_what_is_sent(ctx, monkeypatch):
    monkeypatch.setenv("SURREALFS_MIRROR_ROOT", "/brain")
    sent: list[str] = []

    async def fake(path: str, text: str) -> None:
        sent.append(path)

    # `/brainy` must not pass for `/brain`: a prefix match on the string alone
    # would leak a neighbouring folder into the memory layer.
    for path in ("/brain/in.md", "/brainy/out.md", "/elsewhere/out.md"):
        await mcp.run_tool(
            ctx, "write_file", {"path": path, "content": "x"}, mirror=fake
        )
    assert sent == ["/brain/in.md"]


async def test_spectron_is_a_no_op_when_unconfigured(monkeypatch):
    """Without a key this stays a plain SurrealFS server, and says so.

    `mirror` must not raise -- it runs inside every write -- and `recall` has to
    return text a model can act on rather than an error, since "unconfigured" is
    a legitimate state, not a failure.
    """
    monkeypatch.delenv("SPECTRON_CONTEXT_ID", raising=False)
    monkeypatch.delenv("SPECTRON_API_KEY", raising=False)
    assert not mcp.spectron.configured()

    # No network: a configured Spectron would post to it here.
    assert await mcp.spectron.mirror("/brain/x.md", "text") is None
    assert "not configured" in await mcp.spectron.recall("what is blocking us")


def test_a_hit_with_a_null_score_still_renders():
    """A `"score": null` has the key, so a `.get` default never applies."""
    line = mcp.spectron._render({"source": "chunk", "score": None}, {})
    assert line.startswith("chunk · ? · 0.00")


def test_unexpanded_placeholders_are_refused(monkeypatch):
    monkeypatch.setenv("SURREALDB_URL", "${SURREALDB_URL}")
    assert mcp._unexpanded() == ["SURREALDB_URL"]
    monkeypatch.setenv("SURREALDB_URL", "ws://localhost:8000/rpc")
    assert mcp._unexpanded() == []


def test_config_path_prefers_the_explicit_override(tmp_path, monkeypatch):
    monkeypatch.setenv("SURREALFS_ENV_FILE", str(tmp_path / "custom"))
    assert mcp.config_path() == tmp_path / "custom"

    monkeypatch.delenv("SURREALFS_ENV_FILE")
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path))
    assert mcp.config_path() == tmp_path / "surrealfs" / "env"

    monkeypatch.delenv("XDG_CONFIG_HOME")
    assert mcp.config_path().parts[-3:] == (".config", "surrealfs", "env")


def test_the_config_file_is_a_default_not_a_mandate(tmp_path, monkeypatch):
    """An exported variable has to beat the file.

    Claude Desktop's `env` block and a shell export are both legitimate ways to
    configure this, and the bare-MCP install in the README relies on them.
    """
    config = tmp_path / "env"
    config.write_text(
        "SURREALDB_URL=ws://from-the-file/rpc\n"
        "SPECTRON_SCOPE=filed\n"
        # A secret is copied through verbatim: dotenv interpolation would eat it.
        "SPECTRON_API_KEY=sk-a${b}c\n"
    )
    monkeypatch.setenv("SURREALFS_ENV_FILE", str(config))
    monkeypatch.setenv("SURREALDB_URL", "ws://from-the-environment/rpc")
    monkeypatch.delenv("SPECTRON_SCOPE", raising=False)

    assert mcp._load_config() == config
    import os

    assert os.environ["SURREALDB_URL"] == "ws://from-the-environment/rpc"
    assert os.environ["SPECTRON_SCOPE"] == "filed"
    assert os.environ["SPECTRON_API_KEY"] == "sk-a${b}c"


def test_a_missing_config_file_is_not_an_error(tmp_path, monkeypatch):
    monkeypatch.setenv("SURREALFS_ENV_FILE", str(tmp_path / "absent"))
    assert mcp._load_config() is None
