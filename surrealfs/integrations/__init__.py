"""Ways to hand the SurrealFS tools to an agent.

* :mod:`surrealfs.integrations.json_tools` — plain JSON Schema definitions plus a
  dispatcher. No agent framework required; works with the Anthropic or OpenAI
  SDKs directly.
* :mod:`surrealfs.integrations.pydantic_ai` — a ``FunctionToolset`` for
  pydantic-ai. Requires the ``pydantic-ai`` extra.
* :mod:`surrealfs.integrations.hermes` — a Hermes plugin, discovered through an
  entry point as soon as this package is installed. Depends on nothing.

All three are generated from the registry in :mod:`surrealfs.tools`.

:mod:`surrealfs.integrations.hermes_memory` is the odd one out: a Hermes *memory
provider* rather than a set of tools, so it files turns and recalls them instead of
exposing anything for a model to call. Hermes finds it by directory, so it installs
separately from the plugin above — see its README.
"""

from __future__ import annotations

__all__: list[str] = []
