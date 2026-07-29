"""Ways to hand the SurrealFS tools to an agent.

* :mod:`surrealfs.integrations.json_tools` — plain JSON Schema definitions plus a
  dispatcher. No agent framework required; works with the Anthropic or OpenAI
  SDKs directly.
* :mod:`surrealfs.integrations.pydantic_ai` — a ``FunctionToolset`` for
  pydantic-ai. Requires the ``pydantic-ai`` extra.

Both are generated from the registry in :mod:`surrealfs.tools`.
"""

from __future__ import annotations

__all__: list[str] = []
