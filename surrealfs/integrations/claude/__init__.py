"""The Claude plugin around the MCP server in :mod:`surrealfs.integrations.mcp`.

There is no code here, and that is the point: the server is client-agnostic and
lives next door. This directory holds only what Claude itself needs --
``.claude-plugin/plugin.json``, the bundled ``.mcp.json`` that launches
``surrealfs-mcp``, and ``skills/brain/``.

**Nothing in it may move.** The repo root's ``.claude-plugin/marketplace.json``
points ``source`` straight at this directory, so the manifest, ``.mcp.json`` and
``skills/`` have to sit at *this* root with no duplicated files -- as would
``commands/``, ``agents/`` or ``hooks/`` if they ever appear. Never inside
``.claude-plugin/``. ``tests/test_claude.py`` guards the three JSON files against
drifting apart.

See ``README.md`` beside this file.
"""

from __future__ import annotations

__all__: list[str] = []
