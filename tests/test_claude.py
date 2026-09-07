"""The Claude plugin: three hand-written JSON files and a skill.

The server they point at is `surrealfs.integrations.mcp` and is tested in
`test_mcp.py`; nothing Claude-specific reaches the Python.
"""

from __future__ import annotations

from surrealfs.integrations import claude


def test_the_plugin_manifest_matches_the_marketplace_and_the_skill():
    """The plugin's three hand-written JSON files are the only things that drift."""
    import json
    from pathlib import Path

    root = Path(claude.__file__).parent
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
    assert spec.startswith("surrealfs[mcp] @ ")
    assert "${SURREALFS_SOURCE:-" in spec, "a local checkout must be redirectable"
