"""Ways to hand the SurrealFS tools to an agent.

* :mod:`surrealfs.integrations.json_tools` — plain JSON Schema definitions plus a
  dispatcher. No agent framework required; works with the Anthropic or OpenAI
  SDKs directly.
* :mod:`surrealfs.integrations.pydantic_ai` — a ``FunctionToolset`` for
  pydantic-ai. Requires the ``pydantic-ai`` extra.
* :mod:`surrealfs.integrations.hermes` — a Hermes plugin, discovered through an
  entry point as soon as this package is installed. Depends on nothing.
* :mod:`surrealfs.integrations.mcp` — an MCP server (`surrealfs-mcp`) for any
  client that speaks MCP. Requires the ``mcp`` extra. Configure agent memory and it
  also offers ``brain_recall`` and mirrors every write into it; that half is
  optional and needs the ``agent-memory`` extra plus an API key.
* :mod:`surrealfs.integrations.claude` — the Claude plugin around that server:
  ``.claude-plugin/plugin.json``, the ``.mcp.json`` that launches it, and
  ``skills/brain/`` and ``skills/brain-memory/``. No Python at all, which is the
  point — the server above is client-agnostic.

The first four are generated from the registry in :mod:`surrealfs.tools`;
``claude`` has no tools of its own, being packaging around ``mcp``.

:mod:`surrealfs.integrations.hermes_memory` is the odd one out: a Hermes *memory
provider* rather than a set of tools, so it files turns and recalls them instead of
exposing anything for a model to call. Hermes finds it by directory, so it installs
separately from the plugin above.

Each of the six has a ``README.md`` beside its code; nothing about them is
documented in the root README, which links out instead.
"""

from __future__ import annotations

__all__: list[str] = []
