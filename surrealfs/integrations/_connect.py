"""One connection, opened from the environment, for integrations that host themselves.

Most of this library never connects: the caller owns the handle and passes it in. The
Hermes surfaces are the exception — Hermes hands a plugin a sync callback and no
lifecycle hook to own a connection on, so they have to open their own.

    async with connected() as db:
        await SurrealFs(db).ls("/")

Connection details come from the same ``SURREALDB_*`` variables the rest of the
project uses, defaulting to a local server.
"""

from __future__ import annotations

import os
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

from surrealdb import AsyncSurreal

from ..schema import apply_schema

__all__ = ["connected"]

_schema_applied = False


@asynccontextmanager
async def connected() -> AsyncIterator[AsyncSurreal]:
    """Connect, authenticate, ensure the schema, and close again afterwards.

    One connection per call. Hermes offers no lifecycle hook to hang a long-lived
    one on, and caching would be wrong anyway: its `_run_async` bridge picks a loop
    per call site (persistent in the CLI, per-thread for parallel tools, throwaway
    inside a gateway), and a SurrealDB WebSocket belongs to the loop it was opened
    on. Calls arrive at LLM pace, so the connect cost hides in the noise.
    """
    global _schema_applied

    db = AsyncSurreal(os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"))
    try:
        await db.signin(
            {
                "username": os.environ.get("SURREALDB_USER", "root"),
                "password": os.environ.get("SURREALDB_PASS", "root"),
            }
        )
        await db.use(
            os.environ.get("SURREALDB_NAMESPACE", "surrealfs"),
            os.environ.get("SURREALDB_DATABASE", "demo"),
        )
        if not _schema_applied:
            # Once per process, so a first call works against a bare database
            # without the user running `python -m surrealfs.schema` first.
            await apply_schema(db)
            _schema_applied = True
        yield db
    finally:
        await db.close()
