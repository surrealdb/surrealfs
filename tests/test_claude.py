"""The MCP server for Claude Desktop, and its Spectron mirror."""

from __future__ import annotations

from surrealfs.integrations import claude as mcp
from surrealfs.tools import select_tools


def test_mcp_surface_matches_the_registry():
    """The drift guard: a tool added to the registry has to reach Desktop too."""
    names = [spec["name"] for spec in mcp.tool_specs()]
    assert names == [spec.name for spec in select_tools()] + [mcp.RECALL_TOOL]

    for spec in mcp.tool_specs():
        assert spec["description"].strip(), spec["name"]
        assert spec["schema"].get("type") == "object", spec["name"]

    # Unprefixed, unlike Hermes: an MCP client namespaces tools by server.
    assert not any(name.startswith("surrealfs_") for name in names)


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


def test_recall_is_a_no_op_without_a_spectron(monkeypatch):
    monkeypatch.delenv("SPECTRON_CONTEXT_ID", raising=False)
    monkeypatch.delenv("SPECTRON_API_KEY", raising=False)
    assert not mcp.spectron.configured()


def test_the_plugin_manifest_matches_the_marketplace_and_the_skill():
    """The plugin's three hand-written JSON files are the only things that drift."""
    import json
    from pathlib import Path

    root = Path(mcp.__file__).parent
    manifest = json.loads((root / ".claude-plugin" / "plugin.json").read_text())
    servers = json.loads((root / ".mcp.json").read_text())["mcpServers"]
    market = json.loads(
        (
            Path(__file__).parent.parent / ".claude-plugin" / "marketplace.json"
        ).read_text()
    )

    # Skills live at the plugin root, never inside .claude-plugin/.
    assert (root / "skills" / "brain" / "SKILL.md").is_file()

    entry = next(p for p in market["plugins"] if p["name"] == manifest["name"])
    assert entry["source"].endswith(root.name)
    assert entry["version"] == manifest["version"]

    # No `env` block at all: the server reads its own config file, so none of
    # the `SURREALDB_*` names -- shared with every other SurrealDB tool on the
    # machine -- has to be exported for this one. A block that defaulted them
    # would be worse than none: it would connect to an empty filesystem.
    assert "env" not in servers["surrealfs"], "the config file supersedes this"
    spec = servers["surrealfs"]["args"][1]
    assert spec.startswith("surrealfs[claude] @ ")
    assert "${SURREALFS_SOURCE:-" in spec, "a local checkout must be redirectable"


def test_unexpanded_placeholders_are_refused(monkeypatch):
    monkeypatch.setenv("SURREALDB_URL", "${SURREALDB_URL}")
    assert mcp._unexpanded() == ["SURREALDB_URL"]
    monkeypatch.setenv("SURREALDB_URL", "ws://localhost:8000/rpc")
    assert mcp._unexpanded() == []


def test_parent_key_never_escapes_a_numeric_looking_id():
    """The index key is built by hand because `str(RecordID)` disagrees with it.

    surrealdb-py 3.0.0b8 escapes an id that would otherwise parse as something
    else, while the schema's server-side `<string>` cast never does. An id
    beginning with a digit is the case that diverges, and when it did, `ls`
    silently listed nothing for that one folder.
    """
    from surrealdb import RecordID

    from surrealfs.fs import _parent_key

    numeric = RecordID("file", "3213bqpjgjifypwk6y23")
    assert _parent_key(numeric) == "file:3213bqpjgjifypwk6y23"
    assert _parent_key(RecordID("file", "o39nm9qrdqzss2i7b348")) == (
        "file:o39nm9qrdqzss2i7b348"
    )
    assert _parent_key(None) == "root"


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
    config.write_text("SURREALDB_URL=ws://from-the-file/rpc\nSPECTRON_SCOPE=filed\n")
    monkeypatch.setenv("SURREALFS_ENV_FILE", str(config))
    monkeypatch.setenv("SURREALDB_URL", "ws://from-the-environment/rpc")
    monkeypatch.delenv("SPECTRON_SCOPE", raising=False)

    assert mcp._load_config() == config
    import os

    assert os.environ["SURREALDB_URL"] == "ws://from-the-environment/rpc"
    assert os.environ["SPECTRON_SCOPE"] == "filed"


def test_a_missing_config_file_is_not_an_error(tmp_path, monkeypatch):
    monkeypatch.setenv("SURREALFS_ENV_FILE", str(tmp_path / "absent"))
    assert mcp._load_config() is None
